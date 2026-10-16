//! spg-jit: native code (Cranelift, in-process) for functions whose parameters
//! and return type are `int`, `float`, `bool`, `string`, or an array of one of
//! the first three or of strings (arrays may be parameters, not results).
//!
//! Correctness rule: a native function never produces an error itself. Any case
//! it cannot answer exactly (integer overflow, integer division or modulo by 0
//! or -1, an index out of range, call depth over the VM's limit, a callee that
//! gave up) sets a failure flag and returns; the VM then runs that call itself,
//! which yields the exact same value or the exact same runtime error and stack
//! trace. This is safe because these functions are pure: they never write to
//! an array, and they have no printing, globals or effectful builtins.
//! Float operations never fail in the VM (IEEE results), so they need no guard.
//!
//! Strings and arrays are reference counted, like the VM's `Rc`s. Native code
//! owns one reference for every heap value it holds (a local, a temporary or an
//! argument), and releases it when the value is consumed: on pop, on return,
//! when an operator consumes its operands, and on every failure path. The VM
//! hands over an owned reference for each heap argument and takes ownership of
//! the returned string.
//!
//! It is fed the unoptimized bytecode (the CLI compiles a second, plain copy
//! for this when `--jit` is given), so the instruction set here stays small.
//! The VM itself keeps running the optimized program.
//!
//! ABI: every native function is `(ctx, depth, args...) -> i64`. Floats travel
//! as their 64-bit patterns and heap values as `Rc::into_raw` pointers.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::immediates::Ieee64;
use cranelift_codegen::ir::{
    types::{F64, I128, I64},
    AbiParam, Block, FuncRef, InstBuilder, MemFlags, Signature, Type, UserFuncName, Value as CV,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use spg_typecheck::builtins::BuiltinId;
use spg_vm::chunk::{Chunk, ElemKind, NativeKind as K, OpCode, Program};
use spg_vm::value::Value;
use spg_vm::{NativeCalls, MAX_CALL_DEPTH};

/// offset of the "failed" flag in the context block: [failed, unused]
const CTX_FAILED: i32 = 0;
/// arguments up to this count are supported (keeps the Rust call shim small)
const MAX_ARITY: usize = 4;

// ---- runtime helpers, called from native code ------------------------------
// These run the same Rust operations as the VM, so the results match exactly.
// Heap values are `Rc` references handed over as `i64`; 0 means "none".
// Helpers never panic (a panic cannot unwind through native frames).

/// float remainder and power, as the VM computes them
extern "C" fn spg_fmod(a: f64, b: f64) -> f64 {
    a % b
}

extern "C" fn spg_powf(a: f64, b: f64) -> f64 {
    a.powf(b)
}

extern "C" fn spg_str_retain(p: i64) {
    if p != 0 {
        // SAFETY: `p` came from Rc::into_raw and is alive while we hold a reference
        unsafe { Rc::increment_strong_count(p as *const String) }
    }
}

extern "C" fn spg_str_release(p: i64) {
    if p != 0 {
        // SAFETY: this consumes the reference we hold
        unsafe { drop(Rc::from_raw(p as *const String)) }
    }
}

/// consumes both references and returns a new owned string, `a + b`
extern "C" fn spg_str_concat(a: i64, b: i64) -> i64 {
    // SAFETY: both are owned references from Rc::into_raw; consumed here
    let (a, b) = unsafe { (Rc::from_raw(a as *const String), Rc::from_raw(b as *const String)) };
    // one allocation of the exact size, then two copies; format! would go through the formatter
    let mut joined = String::with_capacity(a.len() + b.len());
    joined.push_str(&a);
    joined.push_str(&b);
    Rc::into_raw(Rc::new(joined)) as i64
}

/// consumes both references and returns 1 if the contents are equal, else 0
extern "C" fn spg_str_eq(a: i64, b: i64) -> i64 {
    // SAFETY: as in spg_str_concat
    let (a, b) = unsafe { (Rc::from_raw(a as *const String), Rc::from_raw(b as *const String)) };
    (*a == *b) as i64
}

/// `len(s)`: character count, as the VM's `Len` builtin. Consumes `p`.
extern "C" fn spg_str_len(p: i64) -> i64 {
    // SAFETY: an owned reference from Rc::into_raw, consumed here
    let s = unsafe { Rc::from_raw(p as *const String) };
    s.chars().count() as i64
}

/// `@into.string` for int, float and bool: the VM's own text for each
extern "C" fn spg_int_str(n: i64) -> i64 {
    Rc::into_raw(Rc::new(n.to_string())) as i64
}

extern "C" fn spg_float_str(f: f64) -> i64 {
    Rc::into_raw(Rc::new(spg_vm::value::format_float(f))) as i64
}

extern "C" fn spg_bool_str(b: i64) -> i64 {
    Rc::into_raw(Rc::new((b != 0).to_string())) as i64
}

extern "C" fn spg_arr_retain(p: i64) {
    if p != 0 {
        // SAFETY: as in spg_str_retain
        unsafe { Rc::increment_strong_count(p as *const RefCell<Vec<Value>>) }
    }
}

extern "C" fn spg_arr_release(p: i64) {
    if p != 0 {
        // SAFETY: as in spg_str_release
        unsafe { drop(Rc::from_raw(p as *const RefCell<Vec<Value>>)) }
    }
}

/// borrows the array (does not consume `p`) and returns its length
extern "C" fn spg_arr_len(p: i64) -> i64 {
    // SAFETY: `p` is alive while native code holds it; we only borrow it
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    arr.try_borrow().map(|v| v.len() as i64).unwrap_or(0)
}

/// element getters borrow the array; the index was checked by native code.
/// A mismatched element cannot happen (arrays are statically typed); it reads as 0.
extern "C" fn spg_arr_get_int(p: i64, i: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    let Ok(v) = arr.try_borrow() else { return 0 };
    match v.get(i as usize) {
        Some(Value::Int(n)) => *n,
        _ => 0,
    }
}

extern "C" fn spg_arr_get_bool(p: i64, i: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    let Ok(v) = arr.try_borrow() else { return 0 };
    match v.get(i as usize) {
        Some(Value::Bool(b)) => *b as i64,
        _ => 0,
    }
}

extern "C" fn spg_arr_get_float(p: i64, i: i64) -> f64 {
    // SAFETY: as in spg_arr_len
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    let Ok(v) = arr.try_borrow() else { return 0.0 };
    match v.get(i as usize) {
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    }
}

/// returns an owned reference to the string element
extern "C" fn spg_arr_get_str(p: i64, i: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    let Ok(v) = arr.try_borrow() else { return 0 };
    match v.get(i as usize) {
        Some(Value::String(s)) => Rc::into_raw(s.clone()) as i64,
        _ => 0,
    }
}

// Undo log for native stores (array elements and struct fields), so a failed native
// call can be rerun by the VM from the state it started in. Stores are logged one by
// one. When the log passes a limit it is compacted into a snapshot of each touched
// object's original contents, so memory grows with the objects, not the writes.
const MIN_LOG: usize = 4096;

/// a heap object whose slots native code can store into: an array, or a struct's fields
#[derive(Clone)]
enum Obj {
    Arr(Rc<RefCell<Vec<Value>>>),
    Rec(spg_vm::value::StructRef),
}

impl Obj {
    fn addr(&self) -> usize {
        match self {
            Obj::Arr(r) => Rc::as_ptr(r) as usize,
            Obj::Rec(r) => Rc::as_ptr(r) as usize,
        }
    }

    /// the slots, mutably borrowed; None when something else holds a borrow
    fn slots(&self) -> Option<std::cell::RefMut<'_, Vec<Value>>> {
        match self {
            Obj::Arr(r) => r.try_borrow_mut().ok(),
            Obj::Rec(r) => r.try_borrow_mut().ok().map(|t| std::cell::RefMut::map(t, |t| &mut t.1)),
        }
    }
}

/// hasher for object addresses: one multiply, instead of SipHash on every store
#[derive(Default)]
struct AddrHasher(u64);

impl std::hash::Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ b as u64;
        }
    }
    fn write_usize(&mut self, n: usize) {
        self.0 = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type AddrSet = std::collections::HashSet<usize, std::hash::BuildHasherDefault<AddrHasher>>;

#[derive(Default)]
struct Undo {
    /// (object, slot, value it replaced), oldest first
    log: Vec<(Obj, usize, Value)>,
    /// the original contents of each object compacted during this call
    snaps: Vec<(Obj, Vec<Value>)>,
    /// the addresses of the objects in `snaps`
    snap_ids: AddrSet,
    /// log size that triggers the next compaction (0 when there is none yet)
    limit: usize,
}

thread_local! {
    static UNDO: RefCell<Undo> = RefCell::new(Undo::default());
}

impl Undo {
    /// turns the log into snapshots of the original contents of the objects it touched
    fn compact(&mut self) {
        let mut copies: std::collections::HashMap<usize, (Obj, Vec<Value>)> = Default::default();
        // newest first: the last undo applied to a copy is the oldest value, the original
        for (o, i, old) in self.log.iter().rev() {
            let p = o.addr();
            if self.snap_ids.contains(&p) {
                continue;
            }
            let (_, copy) = copies.entry(p).or_insert_with(|| {
                let now: Vec<Value> = o.slots().expect("no borrow is active during a native call").to_vec();
                (o.clone(), now)
            });
            copy[*i] = old.clone();
        }
        for (p, (o, copy)) in copies {
            self.snap_ids.insert(p);
            self.snaps.push((o, copy));
        }
        self.log.clear();
        let total: usize = self.snaps.iter().map(|(_, c)| c.len()).sum();
        self.limit = 2 * total;
    }

    /// ends a native call: keeps its stores, or undoes them (the VM reruns the call)
    fn finish(&mut self, keep: bool) {
        if !keep {
            while let Some((o, i, old)) = self.log.pop() {
                if let Some(mut slots) = o.slots() {
                    if let Some(slot) = slots.get_mut(i) {
                        *slot = old;
                    }
                }
            }
            for (o, copy) in self.snaps.drain(..) {
                if let Some(mut slots) = o.slots() {
                    *slots = copy;
                }
            }
        }
        self.log.clear();
        self.snaps.clear();
        self.snap_ids.clear();
        self.limit = 0;
    }
}

/// stores `new` at slot `i` of `o`, logging the value it replaces. Returns 1 when the
/// store cannot be done (bad index, or a borrow is active); `new` is dropped then.
fn store(o: Obj, i: i64, new: Value) -> i64 {
    let old = o.slots().and_then(|mut slots| {
        let slot = slots.get_mut(i as usize)?;
        Some(std::mem::replace(slot, new))
    });
    match old {
        Some(old) => {
            // an object already snapshotted in this call is restored from its snapshot,
            // so its later stores need no entry of their own
            UNDO.with(|u| {
                let mut u = u.borrow_mut();
                if u.snap_ids.contains(&o.addr()) {
                    return;
                }
                u.log.push((o, i as usize, old));
                if u.log.len() >= MIN_LOG.max(u.limit) {
                    u.compact();
                }
            });
            0
        }
        None => 1,
    }
}

/// a store's value, from the 64-bit pattern native code passes: a string is an owned
/// `Rc::into_raw` pointer, which the value takes over
fn value_of(bits: i64, tag: i64) -> Value {
    match tag {
        0 => Value::Int(bits),
        1 => Value::Bool(bits != 0),
        2 => Value::Float(f64::from_bits(bits as u64)),
        // SAFETY: native code passes an owned reference made by Rc::into_raw
        _ => Value::String(unsafe { Rc::from_raw(bits as *const String) }),
    }
}

/// a new reference to the array behind `p` (the caller's reference is left alone)
unsafe fn arr_ref(p: i64) -> Rc<RefCell<Vec<Value>>> {
    Rc::increment_strong_count(p as *const RefCell<Vec<Value>>);
    Rc::from_raw(p as *const RefCell<Vec<Value>>)
}

/// a new reference to the struct behind `p` (the caller's reference is left alone)
unsafe fn rec_ref(p: i64) -> spg_vm::value::StructRef {
    Rc::increment_strong_count(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>);
    Rc::from_raw(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>)
}

/// Stores into `p[i]`. `tag` is the element kind (0 int, 1 bool, 2 float, 3 string).
/// Returns 1 when the store could not be done.
extern "C" fn spg_arr_set(p: i64, i: i64, bits: i64, tag: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    store(Obj::Arr(unsafe { arr_ref(p) }), i, value_of(bits, tag))
}

/// Stores into field `i` of the struct behind `p`; same contract as spg_arr_set.
extern "C" fn spg_struct_set(p: i64, i: i64, bits: i64, tag: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    store(Obj::Rec(unsafe { rec_ref(p) }), i, value_of(bits, tag))
}

/// reads field `i` of the struct behind `p` (borrowed); a string comes back as an
/// owned reference. The kinds are checked by native code, so a mismatch reads as 0.
extern "C" fn spg_struct_get(p: i64, i: i64, tag: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    let rec = unsafe { &*(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>) };
    let Ok(t) = rec.try_borrow() else { return 0 };
    match (t.1.get(i as usize), tag) {
        (Some(Value::Int(n)), 0) => *n,
        (Some(Value::Bool(b)), 1) => *b as i64,
        (Some(Value::Float(f)), 2) => f.to_bits() as i64,
        (Some(Value::String(s)), 3) => Rc::into_raw(s.clone()) as i64,
        _ => 0,
    }
}

extern "C" fn spg_struct_retain(p: i64) {
    if p != 0 {
        // SAFETY: as in spg_str_retain
        unsafe { Rc::increment_strong_count(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>) }
    }
}

extern "C" fn spg_struct_release(p: i64) {
    if p != 0 {
        // SAFETY: as in spg_str_release
        unsafe { drop(Rc::from_raw(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>)) }
    }
}

/// the ownership a native-call argument carries, so a bail-out can release it
#[derive(Clone, Copy, PartialEq)]
enum Owned {
    No,
    Str,
    Arr,
    Obj,
}

fn owned_of(k: K) -> Owned {
    match k {
        K::String => Owned::Str,
        K::Array(_) => Owned::Arr,
        K::Struct(_) => Owned::Obj,
        _ => Owned::No,
    }
}

/// `Some(ptr, arity)` for every function that has native code
pub struct NativeTable {
    _module: JITModule,
    /// keeps the string constants baked into the code alive
    _keep: Vec<Rc<String>>,
    entries: Vec<Option<(usize, usize)>>,
    /// per function, the ownership of each parameter (released on bail-out)
    arg_owned: Vec<Vec<Owned>>,
    /// set after a call to the function gave up. The VM takes over for the rest
    /// of the run: a call that failed deep in a recursion would otherwise be
    /// retried natively at every level the VM re-enters, which is quadratic.
    disabled: Vec<Cell<bool>>,
    /// how many functions were compiled natively (for --time style reporting)
    pub compiled: usize,
}

impl NativeCalls for NativeTable {
    fn has(&self, func_index: usize) -> bool {
        matches!(self.entries.get(func_index), Some(Some(_)))
    }

    fn call(&self, func_index: usize, args: &[i64]) -> Option<i64> {
        let (ptr, arity) = (*self.entries.get(func_index)?)?;
        let owned = &self.arg_owned[func_index];
        if args.len() != arity || self.disabled[func_index].get() {
            // we were handed references the native code will never see: give them back
            for (a, o) in args.iter().zip(owned) {
                match o {
                    Owned::Str => spg_str_release(*a),
                    Owned::Arr => spg_arr_release(*a),
                    Owned::Obj => spg_struct_release(*a),
                    Owned::No => {}
                }
            }
            return None;
        }
        let mut ctx = [0i64; 2];
        let cp = ctx.as_mut_ptr() as i64;
        // every native function takes (ctx, depth, args...); the top call starts at depth 0
        let r = unsafe {
            match arity {
                0 => std::mem::transmute::<usize, extern "C" fn(i64, i64) -> i64>(ptr)(cp, 0),
                1 => std::mem::transmute::<usize, extern "C" fn(i64, i64, i64) -> i64>(ptr)(cp, 0, args[0]),
                2 => std::mem::transmute::<usize, extern "C" fn(i64, i64, i64, i64) -> i64>(ptr)(cp, 0, args[0], args[1]),
                3 => std::mem::transmute::<usize, extern "C" fn(i64, i64, i64, i64, i64) -> i64>(ptr)(cp, 0, args[0], args[1], args[2]),
                _ => std::mem::transmute::<usize, extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64>(ptr)(
                    cp, 0, args[0], args[1], args[2], args[3],
                ),
            }
        };
        // on failure the native code has already released everything it owned,
        // arguments included
        if ctx[0] != 0 {
            // the VM reruns this call, so the stores made before the failure must go
            UNDO.with(|u| u.borrow_mut().finish(false));
            self.disabled[func_index].set(true);
            None
        } else {
            UNDO.with(|u| u.borrow_mut().finish(true));
            Some(r)
        }
    }
}

/// `(parameter kinds, return kind)`
type Sig = (Vec<K>, K);

/// instruction indices that start a block: jump targets, and whatever follows a
/// jump or return (so every terminator ends its block)
fn leaders(chunk: &Chunk) -> BTreeSet<usize> {
    let mut set = BTreeSet::new();
    for (i, op) in chunk.code.iter().enumerate() {
        match op {
            OpCode::Jump(t) | OpCode::JumpIfFalse(t) => {
                set.insert(*t);
                set.insert(i + 1);
            }
            OpCode::Return => {
                set.insert(i + 1);
            }
            _ => {}
        }
    }
    set.remove(&0);
    set.retain(|&i| i < chunk.code.len());
    set
}

/// whether a value of this kind is reference counted
fn is_heap(k: K) -> bool {
    matches!(k, K::String | K::Array(_) | K::Struct(_))
}

/// Type-checks the unoptimized bytecode of one function against the signatures
/// of its callees, and returns the kind of every local slot. Returns `None` for
/// anything the native backend does not support: unsupported opcodes or
/// constants, operands of the wrong kind, a local read before its kind is known,
/// a local that changes kind, values left on the stack across a jump, or a
/// return of the wrong kind.
fn check(chunk: &Chunk, own: &Sig, sigs: &[Option<Sig>], structs: &[Rc<spg_vm::value::StructDef>]) -> Option<Vec<K>> {
    let code = &chunk.code;
    if code.is_empty() || !matches!(code.last(), Some(OpCode::Return) | Some(OpCode::Jump(_))) {
        return None;
    }
    // a jump may land one past the end: that is the "after the function" label. Such a jump
    // only sits in dead code (the `else` of an if whose branches both return), and if it is
    // ever taken, the native call fails and the VM runs the function. Anything further out is refused.
    for (i, op) in code.iter().enumerate() {
        match op {
            OpCode::Jump(t) if *t > code.len() => return None,
            OpCode::JumpIfFalse(t) if *t > code.len() || i + 1 >= code.len() => return None,
            _ => {}
        }
    }
    let leaders = leaders(chunk);
    let mut slots: Vec<Option<K>> = vec![None; chunk.num_locals];
    for (i, k) in own.0.iter().enumerate() {
        *slots.get_mut(i)? = Some(*k);
    }
    let mut st: Vec<K> = Vec::new();

    fn pop(st: &mut Vec<K>, want: K) -> Option<()> {
        (st.pop()? == want).then_some(())
    }
    fn bin(st: &mut Vec<K>, a: K, b: K, out: K) -> Option<()> {
        pop(st, b)?;
        pop(st, a)?;
        st.push(out);
        Some(())
    }
    fn un(st: &mut Vec<K>, a: K, out: K) -> Option<()> {
        pop(st, a)?;
        st.push(out);
        Some(())
    }

    for (i, op) in code.iter().enumerate() {
        if leaders.contains(&i) && !st.is_empty() {
            return None;
        }
        match op {
            OpCode::Const(k) => st.push(match chunk.constants.get(*k)? {
                Value::Int(_) => K::Int,
                Value::Float(_) => K::Float,
                Value::Bool(_) => K::Bool,
                Value::String(_) => K::String,
                _ => return None,
            }),
            OpCode::GetLocal(s) => st.push((*slots.get(*s)?)?),
            OpCode::SetLocal(s) => {
                let k = *st.last()?;
                match slots.get(*s)? {
                    Some(old) if *old != k => return None,
                    _ => slots[*s] = Some(k),
                }
            }
            OpCode::Pop => {
                st.pop()?;
            }
            OpCode::AddInt | OpCode::SubInt | OpCode::MulInt | OpCode::DivInt | OpCode::ModInt => {
                bin(&mut st, K::Int, K::Int, K::Int)?
            }
            OpCode::NegInt => un(&mut st, K::Int, K::Int)?,
            OpCode::AddFloat | OpCode::SubFloat | OpCode::MulFloat | OpCode::DivFloat | OpCode::ModFloat
            | OpCode::PowFloat => bin(&mut st, K::Float, K::Float, K::Float)?,
            OpCode::NegFloat => un(&mut st, K::Float, K::Float)?,
            OpCode::ConcatString => bin(&mut st, K::String, K::String, K::String)?,
            OpCode::EqString | OpCode::NotEqString => bin(&mut st, K::String, K::String, K::Bool)?,
            OpCode::LtInt | OpCode::GtInt | OpCode::LtEqInt | OpCode::GtEqInt | OpCode::EqInt | OpCode::NotEqInt => {
                bin(&mut st, K::Int, K::Int, K::Bool)?
            }
            OpCode::LtFloat | OpCode::GtFloat | OpCode::LtEqFloat | OpCode::GtEqFloat | OpCode::EqFloat
            | OpCode::NotEqFloat => bin(&mut st, K::Float, K::Float, K::Bool)?,
            OpCode::EqBool | OpCode::NotEqBool => bin(&mut st, K::Bool, K::Bool, K::Bool)?,
            OpCode::Not => un(&mut st, K::Bool, K::Bool)?,
            OpCode::IndexGet => {
                // stack: array, index
                pop(&mut st, K::Int)?;
                match st.pop()? {
                    K::Array(e) => st.push(e.value_kind()),
                    _ => return None,
                }
            }
            OpCode::IndexSet => {
                // stack: array, index, value; the value must have the element's kind
                let v = st.pop()?;
                pop(&mut st, K::Int)?;
                match st.pop()? {
                    K::Array(e) if e.value_kind() == v => {}
                    _ => return None,
                }
            }
            OpCode::Dup2 => {
                let n = st.len();
                if n < 2 {
                    return None;
                }
                let (a, c) = (st[n - 2], st[n - 1]);
                st.push(a);
                st.push(c);
            }
            OpCode::FieldGet { field_index } => {
                // stack: struct
                let K::Struct(d) = st.pop()? else { return None };
                let fk = (*structs.get(d)?.field_kinds.get(*field_index)?)?;
                st.push(fk);
            }
            OpCode::FieldSet { field_index } => {
                // stack: struct, value
                let v = st.pop()?;
                let K::Struct(d) = st.pop()? else { return None };
                let fk = (*structs.get(d)?.field_kinds.get(*field_index)?)?;
                if fk != v {
                    return None;
                }
            }
            OpCode::Jump(_) => {
                if !st.is_empty() {
                    return None;
                }
            }
            OpCode::JumpIfFalse(_) => {
                pop(&mut st, K::Bool)?;
                if !st.is_empty() {
                    return None;
                }
            }
            OpCode::Call { func_index, argc } => {
                let (ps, r) = sigs.get(*func_index)?.as_ref()?;
                if *argc != ps.len() || st.len() < *argc {
                    return None;
                }
                let at = st.len() - argc;
                if st[at..] != ps[..] {
                    return None;
                }
                st.truncate(at);
                st.push(*r);
            }
            OpCode::CallBuiltin { id, argc } => {
                // only pure builtins; everything else stays in the VM
                let (args, out): (&[K], K) = match id {
                    BuiltinId::Len => (&[K::String], K::Int),
                    BuiltinId::IntToString => (&[K::Int], K::String),
                    BuiltinId::FloatToString => (&[K::Float], K::String),
                    BuiltinId::BoolToString => (&[K::Bool], K::String),
                    // exact conversions: `as` in Rust, which saturates and maps NaN to 0
                    BuiltinId::IntToFloat => (&[K::Int], K::Float),
                    BuiltinId::FloatToInt => (&[K::Float], K::Int),
                    BuiltinId::ArrayLenBuiltin => {
                        if *argc != 1 || !matches!(st.last(), Some(K::Array(_))) {
                            return None;
                        }
                        st.pop();
                        st.push(K::Int);
                        continue;
                    }
                    _ => return None,
                };
                if *argc != args.len() || st.len() < *argc || st[st.len() - argc..] != *args {
                    return None;
                }
                st.truncate(st.len() - argc);
                st.push(out);
            }
            OpCode::Return => {
                pop(&mut st, own.1)?;
                st.clear();
            }
            // globals, maps, structs, the rest of the builtins: still in the VM
            _ => return None,
        }
    }
    // a local that is never written is never read either; give it a kind anyway
    Some(slots.into_iter().map(|k| k.unwrap_or(K::Int)).collect())
}

/// which functions get native code: a fixpoint over the call graph, so a
/// function that calls a non-native function is not native either
fn eligible(program: &Program) -> (Vec<bool>, Vec<Option<Vec<K>>>) {
    let n = program.functions.len();
    let mut elig: Vec<bool> = program
        .functions
        .iter()
        .map(|c| c.native_sig.is_some() && c.arity <= MAX_ARITY)
        .collect();
    loop {
        let sigs: Vec<Option<Sig>> = (0..n)
            .map(|i| if elig[i] { program.functions[i].native_sig.clone() } else { None })
            .collect();
        let mut changed = false;
        let mut slots = vec![None; n];
        for i in 0..n {
            if !elig[i] {
                continue;
            }
            let own = sigs[i].clone().unwrap();
            match check(&program.functions[i], &own, &sigs, &program.struct_defs) {
                Some(s) => slots[i] = Some(s),
                None => {
                    elig[i] = false;
                    changed = true;
                }
            }
        }
        if !changed {
            return (elig, slots);
        }
    }
}

/// references to the runtime helpers, as seen from one native function
#[derive(Clone, Copy)]
struct Helpers {
    fmod: FuncRef,
    powf: FuncRef,
    str_retain: FuncRef,
    str_release: FuncRef,
    arr_retain: FuncRef,
    arr_release: FuncRef,
    concat: FuncRef,
    streq: FuncRef,
    len: FuncRef,
    int_str: FuncRef,
    float_str: FuncRef,
    bool_str: FuncRef,
    arr_len: FuncRef,
    arr_get_int: FuncRef,
    arr_get_bool: FuncRef,
    arr_get_float: FuncRef,
    arr_get_str: FuncRef,
    arr_set: FuncRef,
    struct_set: FuncRef,
    struct_get: FuncRef,
    struct_retain: FuncRef,
    struct_release: FuncRef,
}

impl Helpers {
    fn retain(&self, b: &mut FunctionBuilder, k: K, v: CV) {
        match k {
            K::String => {
                b.ins().call(self.str_retain, &[v]);
            }
            K::Array(_) => {
                b.ins().call(self.arr_retain, &[v]);
            }
            K::Struct(_) => {
                b.ins().call(self.struct_retain, &[v]);
            }
            _ => {}
        }
    }

    fn release(&self, b: &mut FunctionBuilder, k: K, v: CV) {
        match k {
            K::String => {
                b.ins().call(self.str_release, &[v]);
            }
            K::Array(_) => {
                b.ins().call(self.arr_release, &[v]);
            }
            K::Struct(_) => {
                b.ins().call(self.struct_release, &[v]);
            }
            _ => {}
        }
    }
}

/// heap values on the abstract stack: the ones that must be released when the
/// native call leaves early
fn heap_temps(stack: &[(CV, K)]) -> Vec<(CV, K)> {
    stack.iter().filter(|(_, k)| is_heap(*k)).copied().collect()
}

/// jumps to `fail` when `bad` is non-zero, otherwise continues in a new block.
/// `temps` are released on the way out.
fn guard(b: &mut FunctionBuilder, h: Helpers, bad: CV, fail: Block, temps: Vec<(CV, K)>) {
    let cont = b.create_block();
    if temps.is_empty() {
        b.ins().brif(bad, fail, &[], cont, &[]);
    } else {
        let cleanup = b.create_block();
        b.ins().brif(bad, cleanup, &[], cont, &[]);
        b.switch_to_block(cleanup);
        for (t, k) in temps {
            h.release(b, k, t);
        }
        b.ins().jump(fail, &[]);
    }
    b.switch_to_block(cont);
}

/// emits the "fail the whole native call" exit: release the heap locals, set
/// the flag, return 0
fn fail_block(b: &mut FunctionBuilder, h: Helpers, ctx: CV, fail: Block, heap_vars: &[(Variable, K)]) {
    b.switch_to_block(fail);
    for (v, k) in heap_vars {
        let x = b.use_var(*v);
        h.release(b, *k, x);
    }
    let one = b.ins().iconst(I64, 1);
    b.ins().store(MemFlags::trusted(), one, ctx, CTX_FAILED);
    let zero = b.ins().iconst(I64, 0);
    b.ins().return_(&[zero]);
}

/// the element tag the struct helpers take: 0 int, 1 bool, 2 float, 3 string
fn struct_tag(k: K) -> i64 {
    match k {
        K::Int => 0,
        K::Bool => 1,
        K::Float => 2,
        K::String => 3,
        _ => unreachable!("check() admits only scalar and string fields"),
    }
}

fn cl_type(k: K) -> Type {
    match k {
        K::Float => F64,
        _ => I64,
    }
}

fn lower(
    b: &mut FunctionBuilder,
    chunk: &Chunk,
    own: &Sig,
    slot_kinds: &[K],
    sigs: &[Option<Sig>],
    structs: &[Rc<spg_vm::value::StructDef>],
    refs: &HashMap<usize, FuncRef>,
    h: Helpers,
) {
    let vars: Vec<Variable> = (0..chunk.num_locals).map(|i| Variable::from_u32(i as u32)).collect();
    for (v, k) in vars.iter().zip(slot_kinds) {
        b.declare_var(*v, cl_type(*k));
    }
    let heap_vars: Vec<(Variable, K)> = vars
        .iter()
        .zip(slot_kinds)
        .filter(|(_, k)| is_heap(**k))
        .map(|(v, k)| (*v, *k))
        .collect();

    let fail = b.create_block();
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    b.switch_to_block(entry);
    let params: Vec<CV> = b.block_params(entry).to_vec();
    let ctx = params[0];
    let depth = params[1];
    // parameters arrive owned; the other locals start empty (null / zero)
    for (i, (v, k)) in vars.iter().zip(slot_kinds).enumerate() {
        let raw = if i < chunk.arity {
            params[i + 2]
        } else {
            b.ins().iconst(I64, 0)
        };
        let val = if *k == K::Float { b.ins().bitcast(F64, MemFlags::new(), raw) } else { raw };
        b.def_var(*v, val);
    }

    // depth limit, like the VM's MAX_CALL_DEPTH: deeper calls fall back to the VM.
    // The depth travels in an argument register, not in memory.
    let limit = b.ins().iconst(I64, MAX_CALL_DEPTH as i64);
    let too_deep = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, depth, limit);
    let body = b.create_block();
    b.ins().brif(too_deep, fail, &[], body, &[]);
    b.switch_to_block(body);
    let deeper = b.ins().iadd_imm(depth, 1);

    // one Cranelift block per leader. Instruction 0 gets its own block because a
    // loop may jump back to it.
    let start = b.create_block();
    b.ins().jump(start, &[]);
    b.switch_to_block(start);
    let leader_set = leaders(chunk);
    let mut blocks: HashMap<usize, Block> = HashMap::new();
    blocks.insert(0, start);
    for &i in &leader_set {
        blocks.insert(i, b.create_block());
    }

    let mut stack: Vec<(CV, K)> = Vec::new();
    let mut terminated = false;
    for (i, op) in chunk.code.iter().enumerate() {
        if i != 0 && leader_set.contains(&i) {
            if !terminated {
                b.ins().jump(blocks[&i], &[]);
            }
            b.switch_to_block(blocks[&i]);
            terminated = false;
        }
        match op {
            OpCode::Const(k) => {
                let entry = match &chunk.constants[*k] {
                    Value::Int(n) => (b.ins().iconst(I64, *n), K::Int),
                    Value::Bool(t) => (b.ins().iconst(I64, *t as i64), K::Bool),
                    Value::Float(f) => (b.ins().f64const(Ieee64::with_bits(f.to_bits())), K::Float),
                    Value::String(rc) => {
                        // the constant lives on in the NativeTable; the stack gets its own reference
                        let p = b.ins().iconst(I64, Rc::as_ptr(rc) as usize as i64);
                        h.retain(b, K::String, p);
                        (p, K::String)
                    }
                    _ => unreachable!("check() rejects other constants"),
                };
                stack.push(entry);
            }
            OpCode::GetLocal(s) => {
                let v = b.use_var(vars[*s]);
                let k = slot_kinds[*s];
                h.retain(b, k, v);
                stack.push((v, k));
            }
            OpCode::SetLocal(s) => {
                let (v, k) = *stack.last().unwrap();
                if is_heap(k) {
                    // the slot takes its own reference and drops the one it held
                    h.retain(b, k, v);
                    let old = b.use_var(vars[*s]);
                    h.release(b, k, old);
                }
                b.def_var(vars[*s], v);
            }
            OpCode::Pop => {
                let (v, k) = stack.pop().unwrap();
                h.release(b, k, v);
            }
            OpCode::AddInt | OpCode::SubInt => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let (r, bad) = if matches!(op, OpCode::AddInt) {
                    // signed add overflows iff both operands share a sign the result lacks
                    let r = b.ins().iadd(x, y);
                    let x1 = b.ins().bxor(x, r);
                    let x2 = b.ins().bxor(y, r);
                    let both = b.ins().band(x1, x2);
                    let zero = b.ins().iconst(I64, 0);
                    (r, b.ins().icmp(IntCC::SignedLessThan, both, zero))
                } else {
                    // signed sub overflows iff the operands differ in sign and the result's sign differs from x
                    let r = b.ins().isub(x, y);
                    let x1 = b.ins().bxor(x, y);
                    let x2 = b.ins().bxor(x, r);
                    let both = b.ins().band(x1, x2);
                    let zero = b.ins().iconst(I64, 0);
                    (r, b.ins().icmp(IntCC::SignedLessThan, both, zero))
                };
                guard(b, h, bad, fail, heap_temps(&stack));
                stack.push((r, K::Int));
            }
            OpCode::MulInt => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                // widen to 128 bits: the product fits iff sign-extending its low half gives it back
                let x128 = b.ins().sextend(I128, x);
                let y128 = b.ins().sextend(I128, y);
                let p = b.ins().imul(x128, y128);
                let r = b.ins().ireduce(I64, p);
                let back = b.ins().sextend(I128, r);
                let bad = b.ins().icmp(IntCC::NotEqual, p, back);
                guard(b, h, bad, fail, heap_temps(&stack));
                stack.push((r, K::Int));
            }
            OpCode::DivInt | OpCode::ModInt => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                // by zero, and by -1 (where MIN / -1 overflows): let the VM decide
                let zero = b.ins().iconst(I64, 0);
                let minus1 = b.ins().iconst(I64, -1);
                let z = b.ins().icmp(IntCC::Equal, y, zero);
                let m = b.ins().icmp(IntCC::Equal, y, minus1);
                let bad = b.ins().bor(z, m);
                guard(b, h, bad, fail, heap_temps(&stack));
                let r = if matches!(op, OpCode::DivInt) { b.ins().sdiv(x, y) } else { b.ins().srem(x, y) };
                stack.push((r, K::Int));
            }
            OpCode::NegInt => {
                let (x, _) = stack.pop().unwrap();
                let min = b.ins().iconst(I64, i64::MIN);
                let bad = b.ins().icmp(IntCC::Equal, x, min);
                guard(b, h, bad, fail, heap_temps(&stack));
                let zero = b.ins().iconst(I64, 0);
                stack.push((b.ins().isub(zero, x), K::Int));
            }
            OpCode::AddFloat | OpCode::SubFloat | OpCode::MulFloat | OpCode::DivFloat => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let r = match op {
                    OpCode::AddFloat => b.ins().fadd(x, y),
                    OpCode::SubFloat => b.ins().fsub(x, y),
                    OpCode::MulFloat => b.ins().fmul(x, y),
                    _ => b.ins().fdiv(x, y),
                };
                stack.push((r, K::Float));
            }
            OpCode::ModFloat | OpCode::PowFloat => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let f = if matches!(op, OpCode::ModFloat) { h.fmod } else { h.powf };
                let inst = b.ins().call(f, &[x, y]);
                stack.push((b.inst_results(inst)[0], K::Float));
            }
            OpCode::NegFloat => {
                let (x, _) = stack.pop().unwrap();
                stack.push((b.ins().fneg(x), K::Float));
            }
            OpCode::ConcatString => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                // the helper consumes both operands
                let inst = b.ins().call(h.concat, &[x, y]);
                stack.push((b.inst_results(inst)[0], K::String));
            }
            OpCode::EqString | OpCode::NotEqString => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let inst = b.ins().call(h.streq, &[x, y]);
                let eq = b.inst_results(inst)[0];
                let r = if matches!(op, OpCode::EqString) { eq } else { b.ins().bxor_imm(eq, 1) };
                stack.push((r, K::Bool));
            }
            OpCode::IndexGet => {
                let (idx, _) = stack.pop().unwrap();
                let (arr, ak) = stack.pop().unwrap();
                let elem = match ak {
                    K::Array(e) => e,
                    _ => unreachable!("check() ensures an array"),
                };
                // unsigned compare: a negative index is out of range too
                let len = b.ins().call(h.arr_len, &[arr]);
                let len = b.inst_results(len)[0];
                let bad = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);
                // the array operand is ours to release on the way out
                let mut temps = heap_temps(&stack);
                temps.push((arr, ak));
                guard(b, h, bad, fail, temps);
                let (val, k) = match elem {
                    ElemKind::Int => {
                        let inst = b.ins().call(h.arr_get_int, &[arr, idx]);
                        (b.inst_results(inst)[0], K::Int)
                    }
                    ElemKind::Bool => {
                        let inst = b.ins().call(h.arr_get_bool, &[arr, idx]);
                        (b.inst_results(inst)[0], K::Bool)
                    }
                    ElemKind::Float => {
                        let inst = b.ins().call(h.arr_get_float, &[arr, idx]);
                        (b.inst_results(inst)[0], K::Float)
                    }
                    ElemKind::String => {
                        let inst = b.ins().call(h.arr_get_str, &[arr, idx]);
                        (b.inst_results(inst)[0], K::String)
                    }
                };
                // the element was read by borrow (or cloned for strings); the array reference is spent
                h.release(b, ak, arr);
                stack.push((val, k));
            }
            OpCode::IndexSet => {
                // stack: array, index, value. A string value's reference moves into the array.
                let (val, vk) = stack.pop().unwrap();
                let (idx, _) = stack.pop().unwrap();
                let (arr, ak) = stack.pop().unwrap();
                let elem = match ak {
                    K::Array(e) => e,
                    _ => unreachable!("check() ensures an array"),
                };
                let len = b.ins().call(h.arr_len, &[arr]);
                let len = b.inst_results(len)[0];
                let bad = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);
                let mut temps = heap_temps(&stack);
                temps.push((arr, ak));
                if is_heap(vk) {
                    temps.push((val, vk));
                }
                guard(b, h, bad, fail, temps);
                let (bits, tag) = match elem {
                    ElemKind::Int => (val, 0),
                    ElemKind::Bool => (val, 1),
                    ElemKind::Float => (b.ins().bitcast(I64, MemFlags::new(), val), 2),
                    ElemKind::String => (val, 3),
                };
                let tag = b.ins().iconst(I64, tag);
                let inst = b.ins().call(h.arr_set, &[arr, idx, bits, tag]);
                let status = b.inst_results(inst)[0];
                // the store consumed a string value either way; only a borrow clash lands here
                let mut temps = heap_temps(&stack);
                temps.push((arr, ak));
                guard(b, h, status, fail, temps);
                h.release(b, ak, arr);
            }
            OpCode::Dup2 => {
                // the top two values again; heap ones get their own reference
                let n = stack.len();
                let (a, ak) = stack[n - 2];
                let (c, ck) = stack[n - 1];
                for (v, k) in [(a, ak), (c, ck)] {
                    if is_heap(k) {
                        h.retain(b, k, v);
                    }
                }
                stack.push((a, ak));
                stack.push((c, ck));
            }
            OpCode::FieldGet { field_index } => {
                // stack: struct. The struct reference is ours: released after the read.
                let (s, sk) = stack.pop().unwrap();
                let d = match sk {
                    K::Struct(d) => d,
                    _ => unreachable!("check() ensures a struct"),
                };
                let fk = structs[d].field_kinds[*field_index].unwrap();
                let fi = b.ins().iconst(I64, *field_index as i64);
                let tag = b.ins().iconst(I64, struct_tag(fk));
                let inst = b.ins().call(h.struct_get, &[s, fi, tag]);
                let bits = b.inst_results(inst)[0];
                let v = if fk == K::Float { b.ins().bitcast(F64, MemFlags::new(), bits) } else { bits };
                h.release(b, sk, s);
                stack.push((v, fk));
            }
            OpCode::FieldSet { field_index } => {
                // stack: struct, value. A string value's reference moves into the struct.
                let (val, _) = stack.pop().unwrap();
                let (s, sk) = stack.pop().unwrap();
                let d = match sk {
                    K::Struct(d) => d,
                    _ => unreachable!("check() ensures a struct"),
                };
                let fk = structs[d].field_kinds[*field_index].unwrap();
                let bits = if fk == K::Float { b.ins().bitcast(I64, MemFlags::new(), val) } else { val };
                let fi = b.ins().iconst(I64, *field_index as i64);
                let tag = b.ins().iconst(I64, struct_tag(fk));
                let inst = b.ins().call(h.struct_set, &[s, fi, bits, tag]);
                let status = b.inst_results(inst)[0];
                // the store consumed a string value either way; only a borrow clash lands here
                let mut temps = heap_temps(&stack);
                temps.push((s, sk));
                guard(b, h, status, fail, temps);
                h.release(b, sk, s);
            }
            OpCode::EqBool | OpCode::NotEqBool => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let cc = if matches!(op, OpCode::EqBool) { IntCC::Equal } else { IntCC::NotEqual };
                let c = b.ins().icmp(cc, x, y);
                stack.push((b.ins().uextend(I64, c), K::Bool));
            }
            OpCode::LtInt | OpCode::GtInt | OpCode::LtEqInt | OpCode::GtEqInt | OpCode::EqInt | OpCode::NotEqInt => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let cc = match op {
                    OpCode::LtInt => IntCC::SignedLessThan,
                    OpCode::GtInt => IntCC::SignedGreaterThan,
                    OpCode::LtEqInt => IntCC::SignedLessThanOrEqual,
                    OpCode::GtEqInt => IntCC::SignedGreaterThanOrEqual,
                    OpCode::EqInt => IntCC::Equal,
                    _ => IntCC::NotEqual,
                };
                let c = b.ins().icmp(cc, x, y);
                stack.push((b.ins().uextend(I64, c), K::Bool));
            }
            OpCode::LtFloat | OpCode::GtFloat | OpCode::LtEqFloat | OpCode::GtEqFloat | OpCode::EqFloat
            | OpCode::NotEqFloat => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                // IEEE comparisons, as in the VM: NaN is unequal to everything, so `!=`
                // is true for it. FloatCC::NotEqual is "unordered or not equal".
                let cc = match op {
                    OpCode::LtFloat => FloatCC::LessThan,
                    OpCode::GtFloat => FloatCC::GreaterThan,
                    OpCode::LtEqFloat => FloatCC::LessThanOrEqual,
                    OpCode::GtEqFloat => FloatCC::GreaterThanOrEqual,
                    OpCode::EqFloat => FloatCC::Equal,
                    _ => FloatCC::NotEqual,
                };
                let c = b.ins().fcmp(cc, x, y);
                stack.push((b.ins().uextend(I64, c), K::Bool));
            }
            OpCode::Not => {
                let (x, _) = stack.pop().unwrap();
                stack.push((b.ins().bxor_imm(x, 1), K::Bool));
            }
            OpCode::Jump(t) => {
                // past-the-end target: reaching it means the native code went wrong, so fail
                let target = blocks.get(t).copied().unwrap_or(fail);
                b.ins().jump(target, &[]);
                terminated = true;
            }
            OpCode::JumpIfFalse(t) => {
                let (c, _) = stack.pop().unwrap();
                // brif: non-zero (true) continues, zero (false) jumps to the target
                let next = blocks[&(i + 1)];
                let target = blocks.get(t).copied().unwrap_or(fail);
                b.ins().brif(c, next, &[], target, &[]);
                terminated = true;
            }
            OpCode::Call { func_index, argc } => {
                let at = stack.len() - argc;
                let args: Vec<(CV, K)> = stack.drain(at..).collect();
                let (callee_ps, callee_r) = sigs[*func_index].clone().unwrap();
                // callees take every argument as its 64-bit pattern; the arguments are consumed
                let mut call_args = vec![ctx, deeper];
                for ((v, _), k) in args.iter().zip(callee_ps) {
                    call_args.push(if k == K::Float { b.ins().bitcast(I64, MemFlags::new(), *v) } else { *v });
                }
                let fref = refs[func_index];
                let inst = b.ins().call(fref, &call_args);
                let raw = b.inst_results(inst)[0];
                // a callee that failed left its flag set; its result is meaningless
                let failed = b.ins().load(I64, MemFlags::trusted(), ctx, CTX_FAILED);
                guard(b, h, failed, fail, heap_temps(&stack));
                let val = if callee_r == K::Float { b.ins().bitcast(F64, MemFlags::new(), raw) } else { raw };
                stack.push((val, callee_r));
            }
            OpCode::CallBuiltin { id, argc } => {
                let at = stack.len() - argc;
                let args: Vec<(CV, K)> = stack.drain(at..).collect();
                let x = args[0].0;
                if matches!(id, BuiltinId::IntToFloat) {
                    stack.push((b.ins().fcvt_from_sint(F64, x), K::Float));
                    continue;
                }
                if matches!(id, BuiltinId::FloatToInt) {
                    stack.push((b.ins().fcvt_to_sint_sat(I64, x), K::Int));
                    continue;
                }
                let (f, out) = match id {
                    BuiltinId::Len => (h.len, K::Int),
                    BuiltinId::IntToString => (h.int_str, K::String),
                    BuiltinId::FloatToString => (h.float_str, K::String),
                    BuiltinId::BoolToString => (h.bool_str, K::String),
                    _ => {
                        // ArrayLenBuiltin: count, then the array reference is spent
                        let inst = b.ins().call(h.arr_len, &[x]);
                        let n = b.inst_results(inst)[0];
                        h.release(b, args[0].1, x);
                        stack.push((n, K::Int));
                        continue;
                    }
                };
                let inst = b.ins().call(f, &[x]);
                stack.push((b.inst_results(inst)[0], out));
            }
            OpCode::Return => {
                let (v, _) = stack.pop().unwrap();
                // the value is handed to the caller; everything else we still own goes
                for (var, k) in &heap_vars {
                    let x = b.use_var(*var);
                    h.release(b, *k, x);
                }
                for (t, k) in heap_temps(&stack) {
                    h.release(b, k, t);
                }
                let v = if own.1 == K::Float { b.ins().bitcast(I64, MemFlags::new(), v) } else { v };
                b.ins().return_(&[v]);
                terminated = true;
            }
            _ => unreachable!("check() rejects unsupported opcodes"),
        }
    }
    fail_block(b, h, ctx, fail, &heap_vars);
    b.seal_all_blocks();
}

/// Compiles every eligible function of `program`. Errors only if Cranelift
/// refuses the code; the VM then keeps running everything.
pub fn build(program: &Program) -> Result<NativeTable, String> {
    let (elig, slots) = eligible(program);
    let sigs: Vec<Option<Sig>> = (0..program.functions.len())
        .map(|i| if elig[i] { program.functions[i].native_sig.clone() } else { None })
        .collect();

    // string constants are baked into the code by address: keep them alive
    let mut keep: Vec<Rc<String>> = Vec::new();
    for c in &program.functions {
        for v in &c.constants {
            if let Value::String(rc) = v {
                keep.push(rc.clone());
            }
        }
    }

    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    // the platform's C calling convention: SysV on Linux, Windows x64 on Windows. It must match
    // the Rust `extern "C"` helpers and the `extern "C"` entry point we call native code through.
    let call_conv = isa.default_call_conv();
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    let symbols: &[(&str, *const u8)] = &[
        ("spg_fmod", spg_fmod as *const u8),
        ("spg_powf", spg_powf as *const u8),
        ("spg_str_retain", spg_str_retain as *const u8),
        ("spg_str_release", spg_str_release as *const u8),
        ("spg_str_concat", spg_str_concat as *const u8),
        ("spg_str_eq", spg_str_eq as *const u8),
        ("spg_str_len", spg_str_len as *const u8),
        ("spg_int_str", spg_int_str as *const u8),
        ("spg_float_str", spg_float_str as *const u8),
        ("spg_bool_str", spg_bool_str as *const u8),
        ("spg_arr_retain", spg_arr_retain as *const u8),
        ("spg_arr_release", spg_arr_release as *const u8),
        ("spg_arr_len", spg_arr_len as *const u8),
        ("spg_arr_get_int", spg_arr_get_int as *const u8),
        ("spg_arr_get_bool", spg_arr_get_bool as *const u8),
        ("spg_arr_get_float", spg_arr_get_float as *const u8),
        ("spg_arr_get_str", spg_arr_get_str as *const u8),
        ("spg_arr_set", spg_arr_set as *const u8),
        ("spg_struct_set", spg_struct_set as *const u8),
        ("spg_struct_get", spg_struct_get as *const u8),
        ("spg_struct_retain", spg_struct_retain as *const u8),
        ("spg_struct_release", spg_struct_release as *const u8),
    ];
    for (name, ptr) in symbols {
        jb.symbol(*name, *ptr);
    }
    let mut module = JITModule::new(jb);

    let sig = |params: &[Type], ret: Option<Type>| {
        let mut s = Signature::new(call_conv);
        for p in params {
            s.params.push(AbiParam::new(*p));
        }
        if let Some(r) = ret {
            s.returns.push(AbiParam::new(r));
        }
        s
    };
    let imp = |module: &mut JITModule, name: &str, s: &Signature| {
        module.declare_function(name, Linkage::Import, s).map_err(|e| e.to_string())
    };
    let f2f = sig(&[F64, F64], Some(F64));
    let f1i = sig(&[F64], Some(I64));
    let i1v = sig(&[I64], None);
    let i1i = sig(&[I64], Some(I64));
    let i2i = sig(&[I64, I64], Some(I64));
    let i2f = sig(&[I64, I64], Some(F64));
    let i4i = sig(&[I64, I64, I64, I64], Some(I64));
    let i3i = sig(&[I64, I64, I64], Some(I64));
    let decls: Vec<(&str, FuncId)> = vec![
        ("spg_fmod", imp(&mut module, "spg_fmod", &f2f)?),
        ("spg_powf", imp(&mut module, "spg_powf", &f2f)?),
        ("spg_str_retain", imp(&mut module, "spg_str_retain", &i1v)?),
        ("spg_str_release", imp(&mut module, "spg_str_release", &i1v)?),
        ("spg_str_concat", imp(&mut module, "spg_str_concat", &i2i)?),
        ("spg_str_eq", imp(&mut module, "spg_str_eq", &i2i)?),
        ("spg_str_len", imp(&mut module, "spg_str_len", &i1i)?),
        ("spg_int_str", imp(&mut module, "spg_int_str", &i1i)?),
        ("spg_float_str", imp(&mut module, "spg_float_str", &f1i)?),
        ("spg_bool_str", imp(&mut module, "spg_bool_str", &i1i)?),
        ("spg_arr_retain", imp(&mut module, "spg_arr_retain", &i1v)?),
        ("spg_arr_release", imp(&mut module, "spg_arr_release", &i1v)?),
        ("spg_arr_len", imp(&mut module, "spg_arr_len", &i1i)?),
        ("spg_arr_get_int", imp(&mut module, "spg_arr_get_int", &i2i)?),
        ("spg_arr_get_bool", imp(&mut module, "spg_arr_get_bool", &i2i)?),
        ("spg_arr_get_float", imp(&mut module, "spg_arr_get_float", &i2f)?),
        ("spg_arr_get_str", imp(&mut module, "spg_arr_get_str", &i2i)?),
        ("spg_arr_set", imp(&mut module, "spg_arr_set", &i4i)?),
        ("spg_struct_set", imp(&mut module, "spg_struct_set", &i4i)?),
        ("spg_struct_get", imp(&mut module, "spg_struct_get", &i3i)?),
        ("spg_struct_retain", imp(&mut module, "spg_struct_retain", &i1v)?),
        ("spg_struct_release", imp(&mut module, "spg_struct_release", &i1v)?),
    ];
    let id_of = |name: &str| decls.iter().find(|(n, _)| *n == name).unwrap().1;

    // every native function has the same ABI: (ctx, depth, args as i64...) -> i64
    let abi = |arity: usize| {
        let mut s = Signature::new(call_conv);
        for _ in 0..arity + 2 {
            s.params.push(AbiParam::new(I64));
        }
        s.returns.push(AbiParam::new(I64));
        s
    };

    let mut ids: HashMap<usize, FuncId> = HashMap::new();
    for (i, chunk) in program.functions.iter().enumerate() {
        if !elig[i] {
            continue;
        }
        let id = module
            .declare_function(&format!("spg_fn_{i}"), Linkage::Local, &abi(chunk.arity))
            .map_err(|e| e.to_string())?;
        ids.insert(i, id);
    }

    let mut fctx = Context::new();
    let mut fbc = FunctionBuilderContext::new();
    for (i, chunk) in program.functions.iter().enumerate() {
        let Some(&id) = ids.get(&i) else { continue };
        fctx.clear();
        fctx.func.signature = abi(chunk.arity);
        fctx.func.name = UserFuncName::user(0, id.as_u32());
        // references must be declared in this function before its builder starts
        let mut refs: HashMap<usize, FuncRef> = HashMap::new();
        for (&callee, &cid) in &ids {
            if calls(chunk, callee) {
                refs.insert(callee, module.declare_func_in_func(cid, &mut fctx.func));
            }
        }
        let mut declare = |name: &str| module.declare_func_in_func(id_of(name), &mut fctx.func);
        let h = Helpers {
            fmod: declare("spg_fmod"),
            powf: declare("spg_powf"),
            str_retain: declare("spg_str_retain"),
            str_release: declare("spg_str_release"),
            arr_retain: declare("spg_arr_retain"),
            arr_release: declare("spg_arr_release"),
            concat: declare("spg_str_concat"),
            streq: declare("spg_str_eq"),
            len: declare("spg_str_len"),
            int_str: declare("spg_int_str"),
            float_str: declare("spg_float_str"),
            bool_str: declare("spg_bool_str"),
            arr_len: declare("spg_arr_len"),
            arr_get_int: declare("spg_arr_get_int"),
            arr_get_bool: declare("spg_arr_get_bool"),
            arr_get_float: declare("spg_arr_get_float"),
            arr_get_str: declare("spg_arr_get_str"),
            arr_set: declare("spg_arr_set"),
            struct_set: declare("spg_struct_set"),
            struct_get: declare("spg_struct_get"),
            struct_retain: declare("spg_struct_retain"),
            struct_release: declare("spg_struct_release"),
        };
        {
            let mut b = FunctionBuilder::new(&mut fctx.func, &mut fbc);
            let own = sigs[i].clone().unwrap();
            lower(&mut b, chunk, &own, slots[i].as_ref().unwrap(), &sigs, &program.struct_defs, &refs, h);
            b.finalize();
        }
        module.define_function(id, &mut fctx).map_err(|e| format!("{}: {e}", chunk.name))?;
        module.clear_context(&mut fctx);
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;

    let mut entries = vec![None; program.functions.len()];
    let mut arg_owned = vec![Vec::new(); program.functions.len()];
    for (&i, &id) in &ids {
        let ptr = module.get_finalized_function(id) as usize;
        entries[i] = Some((ptr, program.functions[i].arity));
        arg_owned[i] = sigs[i].as_ref().unwrap().0.iter().map(|k| owned_of(*k)).collect();
    }
    let disabled = (0..program.functions.len()).map(|_| Cell::new(false)).collect();
    let compiled = ids.len();
    Ok(NativeTable { _module: module, _keep: keep, entries, arg_owned, disabled, compiled })
}

fn calls(chunk: &Chunk, callee: usize) -> bool {
    chunk.code.iter().any(|op| matches!(op, OpCode::Call { func_index, .. } if *func_index == callee))
}
