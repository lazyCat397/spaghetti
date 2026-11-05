//! spg-jit: native code (Cranelift, in-process) for functions whose params
//! and return type are `int`, `float`, `bool`, `string`, or an array of one of
//! the first three or of strings (arrays may be params, not results).
//!
//! correctness rule: a native function never produces an error itself. any case
//! it can't answer exactly (integer overflow, int division or modulo by 0
//! or -1, an index out of range, call depth over the VM's limit, a callee that
//! gave up) sets a failure flag and returns. the VM then runs that call itself,
//! which gives the exact same value, or the exact same error and stack
//! trace. this is safe because these functions are pure. they never write to
//! an array, and they have no printing, globals or effectful builtins. boring = safe.
//! float ops never fail in the VM (IEEE results), so no guard needed. floats, being floats.
//!
//! strings and arrays are reference counted, like the VM's `Rc`s. native code
//! owns one reference for every heap value it holds (a local, a temp or an
//! arg), and releases it when the value gets consumed: on pop, on return,
//! when an operator eats its operands, and on every failure path. the VM
//! hands over an owned reference for each heap arg, and takes ownership of
//! the returned string. the paperwork is done.
//!
//! it gets fed the unoptimized bytecode (the CLI compiles a second, plain copy
//! for this when `--jit` is given), so the instruction set here stays small.
//! the VM itself keeps running the optimized program. not my problem.
//!
//! ABI: every native function is `(ctx, depth, args...) -> i64`. floats travel
//! as their 64-bit patterns, heap values as `Rc::into_raw` pointers. fun.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::ir::immediates::Ieee64;
use cranelift_codegen::ir::{
    immediates::Offset32,
    types::{F64, I128, I64},
    AbiParam, Block, FuncRef, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind, Type, UserFuncName,
    Value as CV,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use spg_typecheck::builtins::BuiltinId;
use spg_vm::chunk::{Chunk, ElemKind, NativeKind as K, OpCode, Program};
use spg_vm::value::Value;
use spg_vm::vm::LoopOutcome;
use spg_vm::{call_depth_limit, NativeCalls};

/// offset of the "failed" flag in the context block: [failed, unused]
const CTX_FAILED: i32 = 0;
/// args up to this count are supported (keeps the Rust call shim small)
const MAX_ARITY: usize = 4;

// ---- runtime helpers, called from native code (the Rust side of the deal) ------
// these run the same Rust ops as the VM, so the results match exactly.
// heap values are `Rc` references handed over as `i64`. 0 means "none".
// helpers never panic. a panic can't unwind through native frames. please, no panics.

/// float remainder and power, the way the VM computes them
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
        // SAFETY: this consumes the reference we hold. one-way ticket.
        unsafe { drop(Rc::from_raw(p as *const String)) }
    }
}

/// consumes both references and returns a new owned string, `a + b`
extern "C" fn spg_str_concat(a: i64, b: i64) -> i64 {
    // SAFETY: both are owned references from Rc::into_raw; consumed here
    let (a, b) = unsafe { (Rc::from_raw(a as *const String), Rc::from_raw(b as *const String)) };
    // one allocation of the exact size, then two copies. format! would go through the formatter, and that's slow.
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

/// `len(s)`: character count, like the VM's `Len` builtin. consumes `p`.
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

thread_local! {
    /// the `print`s of native code that hasn't settled yet, one batch of argument values per call
    static PENDING_OUTPUT: RefCell<Vec<Vec<spg_vm::value::Value>>> = RefCell::new(Vec::new());
}

/// `print` from native code. the arguments are queued, not written: the output is written only when the
/// native code that made it finishes (see `settle_output`), since a region that gives up is run again by the VM,
/// and that run prints the same things itself
extern "C" fn spg_print(buf: *const i64, argc: i64, kinds: i64) {
    let mut vals = Vec::with_capacity(argc as usize);
    for i in 0..argc as usize {
        // SAFETY: the code stored `argc` values at `buf`, and the kinds say how to read each
        let bits = unsafe { *buf.add(i) };
        vals.push(match (kinds >> (2 * i)) & 3 {
            0 => Value::Int(bits),
            1 => Value::Float(f64::from_bits(bits as u64)),
            2 => Value::Bool(bits != 0),
            _ => {
                // SAFETY: a string argument is a live reference of the native code: the value takes its own count
                unsafe { Rc::increment_strong_count(bits as *const String) };
                Value::String(unsafe { Rc::from_raw(bits as *const String) })
            }
        });
    }
    PENDING_OUTPUT.with(|p| p.borrow_mut().push(vals));
}

/// writes the queued output of native code that finished, or drops it when that code gave up
fn settle_output(finished: bool) {
    let batches = PENDING_OUTPUT.with(|p| std::mem::take(&mut *p.borrow_mut()));
    if finished {
        for vals in &batches {
            spg_vm::builtins::print_values(vals);
        }
    }
}

/// borrows the array (doesn't consume `p`) and returns its length
extern "C" fn spg_arr_len(p: i64) -> i64 {
    // SAFETY: `p` is alive while native code holds it. we only borrow it.
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    arr.try_borrow().map(|v| v.len() as i64).unwrap_or(0)
}

/// element getters borrow the array. native code already checked the index.
/// a mismatched element can't happen (arrays are statically typed). it reads as 0.
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

// undo log for native stores (array elements and struct fields), so a failed native
// call can be rerun by the VM from the state it started in. stores get logged one by
// one. when the log passes a limit it gets compacted into a snapshot of each touched
// object's original contents. memory grows with the objects, not the writes. clever.
const MIN_LOG: usize = 4096;

/// undo marker: the entry is a push, so undoing it removes the last element
const PUSHED: usize = usize::MAX;

/// a heap object native code can store into: an array, or a struct's fields
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

    /// the slots, mutably borrowed. None when something else holds a borrow
    fn slots(&self) -> Option<std::cell::RefMut<'_, Vec<Value>>> {
        // read-only (a const): None sends the call back to the VM, which raises the error
        if spg_vm::frozen::is_frozen_addr(self.addr()) {
            return None;
        }
        match self {
            Obj::Arr(r) => r.try_borrow_mut().ok(),
            Obj::Rec(r) => r.try_borrow_mut().ok().map(|t| std::cell::RefMut::map(t, |t| &mut t.1)),
        }
    }
}

/// hasher for object addresses: one multiply, instead of SipHash on every store. fast hashing, no crypto needed.
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
    /// log size that triggers the next compaction (0 when there's none yet)
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
            // a push is undone by dropping the element it added
            if *i == PUSHED {
                copy.pop();
            } else {
                copy[*i] = old.clone();
            }
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
                    if i == PUSHED {
                        slots.pop();
                    } else if let Some(slot) = slots.get_mut(i) {
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

/// stores `new` at slot `i` of `o`, logging the value it replaces. returns 1 when the
/// store can't be done (bad index, or a borrow is active). `new` gets dropped then.
fn store(o: Obj, i: i64, new: Value) -> i64 {
    let old = o.slots().and_then(|mut slots| {
        let slot = slots.get_mut(i as usize)?;
        Some(std::mem::replace(slot, new))
    });
    match old {
        Some(old) => {
            // an object already snapshotted in this call gets restored from its snapshot,
            // so its later stores need no entry of their own. efficient, like a lazy genius.
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

/// a store's value, from the 64-bit pattern native code passes. a string is an owned
/// `Rc::into_raw` pointer, which the value takes over
fn value_of(bits: i64, tag: i64) -> Value {
    match tag {
        0 => Value::Int(bits),
        1 => Value::Bool(bits != 0),
        2 => Value::Float(f64::from_bits(bits as u64)),
        t if t >= 4 => Value::Enum((t - 4) as u32, bits as u32),
        // SAFETY: native code passes an owned reference made by Rc::into_raw
        _ => Value::String(unsafe { Rc::from_raw(bits as *const String) }),
    }
}

/// a new reference to the array behind `p` (the caller's ref is left alone)
unsafe fn arr_ref(p: i64) -> Rc<RefCell<Vec<Value>>> {
    Rc::increment_strong_count(p as *const RefCell<Vec<Value>>);
    Rc::from_raw(p as *const RefCell<Vec<Value>>)
}

/// a new reference to the struct behind `p` (the caller's ref is left alone)
unsafe fn rec_ref(p: i64) -> spg_vm::value::StructRef {
    Rc::increment_strong_count(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>);
    Rc::from_raw(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>)
}

/// stores into `p[i]`. `tag` is the element kind (0 int, 1 bool, 2 float, 3 string).
/// returns 1 when the store couldn't be done.
extern "C" fn spg_arr_set(p: i64, i: i64, bits: i64, tag: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    store(Obj::Arr(unsafe { arr_ref(p) }), i, value_of(bits, tag))
}

/// typed local arrays (see the typed mode in `eligible`): plain bits, refcounted like the others.
/// they never reach the VM, so no undo entries. helpers never panic (no unwinding through native code).
extern "C" fn spg_tarr_new() -> i64 {
    Rc::into_raw(Rc::new(RefCell::new(Vec::<i64>::new()))) as i64
}

extern "C" fn spg_tarr_retain(p: i64) {
    if p != 0 {
        // SAFETY: p came from Rc::into_raw and is alive while native code holds it
        unsafe { Rc::increment_strong_count(p as *const RefCell<Vec<i64>>) }
    }
}

extern "C" fn spg_tarr_release(p: i64) {
    if p != 0 {
        // SAFETY: consumes the reference native code holds
        unsafe { drop(Rc::from_raw(p as *const RefCell<Vec<i64>>)) }
    }
}

extern "C" fn spg_tarr_len(p: i64) -> i64 {
    // SAFETY: as in spg_tarr_retain. we only borrow it
    let arr = unsafe { &*(p as *const RefCell<Vec<i64>>) };
    arr.try_borrow().map(|v| v.len() as i64).unwrap_or(0)
}

extern "C" fn spg_tarr_get(p: i64, i: i64) -> i64 {
    // SAFETY: as in spg_tarr_len. native code checked the index already
    let arr = unsafe { &*(p as *const RefCell<Vec<i64>>) };
    arr.try_borrow().ok().and_then(|v| v.get(i as usize).copied()).unwrap_or(0)
}

extern "C" fn spg_tarr_set(p: i64, i: i64, bits: i64) -> i64 {
    // SAFETY: as in spg_tarr_len
    let arr = unsafe { &*(p as *const RefCell<Vec<i64>>) };
    if let Ok(mut v) = arr.try_borrow_mut() {
        if let Some(slot) = v.get_mut(i as usize) {
            *slot = bits;
        }
    }
    0
}

extern "C" fn spg_tarr_push(p: i64, bits: i64) -> i64 {
    // SAFETY: as in spg_tarr_len
    let arr = unsafe { &*(p as *const RefCell<Vec<i64>>) };
    if let Ok(mut v) = arr.try_borrow_mut() {
        v.push(bits);
    }
    0
}

/// a new, empty array: an owned reference
extern "C" fn spg_arr_new() -> i64 {
    Rc::into_raw(Rc::new(RefCell::new(Vec::<Value>::new()))) as i64
}

/// appends to an array native code just made (nothing can observe it yet, so no undo entry).
/// consumes the element.
extern "C" fn spg_arr_add(p: i64, bits: i64, tag: i64) -> i64 {
    let value = value_of(bits, tag);
    // SAFETY: p is the owned reference native code holds. we only borrow it
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    if let Ok(mut v) = arr.try_borrow_mut() {
        v.push(value);
    }
    0
}

/// `push` on an array the program already has (borrowed). logged for undo, so a failed
/// native call can take the element back out. consumes the element. returns 1 when the
/// array is const: the VM raises the error.
extern "C" fn spg_arr_push(p: i64, bits: i64, tag: i64) -> i64 {
    let value = value_of(bits, tag);
    let o = Obj::Arr(unsafe { arr_ref(p) });
    let pushed = match o.slots() {
        Some(mut slots) => {
            slots.push(value);
            true
        }
        None => false,
    };
    if !pushed {
        return 1;
    }
    UNDO.with(|u| {
        let mut u = u.borrow_mut();
        if !u.snap_ids.contains(&o.addr()) {
            u.log.push((o, PUSHED, Value::Void));
        }
    });
    0
}

/// stores into an array native code just made: no undo entry needed. the index was checked already.
extern "C" fn spg_arr_put(p: i64, i: i64, bits: i64, tag: i64) -> i64 {
    let value = value_of(bits, tag);
    // SAFETY: as in spg_arr_add
    let arr = unsafe { &*(p as *const RefCell<Vec<Value>>) };
    if let Ok(mut v) = arr.try_borrow_mut() {
        if let Some(slot) = v.get_mut(i as usize) {
            *slot = value;
        }
    }
    0
}

/// stores into field `i` of the struct behind `p`. same contract as spg_arr_set.
extern "C" fn spg_struct_set(p: i64, i: i64, bits: i64, tag: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    store(Obj::Rec(unsafe { rec_ref(p) }), i, value_of(bits, tag))
}

/// reads field `i` of the struct behind `p` (borrowed). a string comes back as an
/// owned reference. native code checks the kinds, so a mismatch reads as 0.
extern "C" fn spg_struct_get(p: i64, i: i64, tag: i64) -> i64 {
    // SAFETY: as in spg_arr_len
    let rec = unsafe { &*(p as *const RefCell<(Rc<spg_vm::value::StructDef>, Vec<Value>)>) };
    let Ok(t) = rec.try_borrow() else { return 0 };
    match (t.1.get(i as usize), tag) {
        (Some(Value::Int(n)), 0) => *n,
        (Some(Value::Bool(b)), 1) => *b as i64,
        (Some(Value::Float(f)), 2) => f.to_bits() as i64,
        (Some(Value::String(s)), 3) => Rc::into_raw(s.clone()) as i64,
        (Some(Value::Enum(d, v)), t) if t == 4 + *d as i64 => *v as i64,
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

/// makes a struct of the type behind `def` from `n` field patterns at `buf`, in field order.
/// `def` is the address of a `StructDef` the native table keeps alive. a string field's owned
/// reference moves into the struct. native code checked the kinds against the def, so this
/// can't fail. returns an owned reference.
extern "C" fn spg_struct_new(def: i64, buf: i64, n: i64) -> i64 {
    // SAFETY: `def` is Rc::as_ptr of a def that the native table holds a reference to for as long
    // as the code lives. `buf` holds `n` words the caller wrote just before the call
    let def = unsafe {
        Rc::increment_strong_count(def as *const spg_vm::value::StructDef);
        Rc::from_raw(def as *const spg_vm::value::StructDef)
    };
    let words = unsafe { std::slice::from_raw_parts(buf as *const i64, n as usize) };
    let fields: Vec<Value> = words
        .iter()
        .zip(def.field_kinds.iter())
        .map(|(&bits, k)| match k {
            Some(K::Int) => Value::Int(bits),
            Some(K::Bool) => Value::Bool(bits != 0),
            Some(K::Float) => Value::Float(f64::from_bits(bits as u64)),
            Some(K::Enum(d)) => Value::Enum(*d as u32, bits as u32),
            // SAFETY: an owned reference made by Rc::into_raw, as in value_of
            Some(K::String) => Value::String(unsafe { Rc::from_raw(bits as *const String) }),
            // unreachable: check() only admits the kinds above
            _ => Value::Int(0),
        })
        .collect();
    Rc::into_raw(Rc::new(RefCell::new((def, fields)))) as i64
}

/// the ownership a native-call arg carries, so a bail-out can release it
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
    module: RefCell<JITModule>,
    /// the plain program: loop regions are compiled from it, at run time
    plain: Program,
    ids: HashMap<usize, FuncId>,
    sigs: Vec<Option<Sig>>,
    structs: Vec<Rc<spg_vm::value::StructDef>>,
    decls: Vec<(&'static str, FuncId)>,
    call_conv: CallConv,
    /// loop regions by (chunk, exit): None when native can't run it
    regions: RefCell<HashMap<(usize, usize), Option<Rc<Region>>>>,
    /// how many times each loop top was reached, until it's worth compiling
    counts: RefCell<HashMap<(usize, usize), u32>>,
    next_region: Cell<usize>,
    /// keeps the string constants baked into the code alive. don't let them die.
    _keep: Vec<Rc<String>>,
    entries: Vec<Option<(usize, usize)>>,
    /// per function, the ownership of each param (released on bail-out)
    arg_owned: Vec<Vec<Owned>>,
    /// set after a call to the function gave up. the VM takes over for the rest
    /// of the run. a call that failed deep in a recursion would otherwise be
    /// retried natively at every level the VM re-enters. quadratic. no thanks.
    disabled: Vec<Cell<bool>>,
    /// how many functions got compiled natively (for --time reporting)
    pub compiled: usize,
    /// why each function is not native (None when it is)
    reasons: Vec<Option<Refusal>>,
    /// why each loop that was tried is not native, by (chunk, exit)
    refused: RefCell<HashMap<(usize, usize), Refusal>>,
    /// the native code of every `compile JIT` block, by (chunk, block)
    blocks: RefCell<HashMap<(usize, usize), Rc<Region>>>,
}

impl NativeCalls for NativeTable {
    fn report_lines(&self) -> Vec<String> {
        self.build_report()
    }

    fn run_block(&self, chunk: usize, block: usize, locals: &mut [Value]) -> spg_vm::vm::BlockOutcome {
        use spg_vm::vm::BlockOutcome;
        let Some(region) = self.blocks.borrow().get(&(chunk, block)).cloned() else {
            return BlockOutcome::Off;
        };
        if locals.len() != region.nlocals {
            return BlockOutcome::Failed;
        }
        let mut buf = vec![0i64; region.nlocals];
        for &(s, k) in &region.loads {
            buf[s] = match (k, &locals[s]) {
                (K::Int, Value::Int(n)) => *n,
                (K::Float, Value::Float(f)) => f.to_bits() as i64,
                (K::Bool, Value::Bool(b)) => *b as i64,
                (K::Array(_), Value::Array(_)) | (K::String, Value::String(_)) => 0,
                _ => return BlockOutcome::Failed,
            };
        }
        // the region takes its own reference to each array or string it reads, as a loop region does
        for &(s, k) in &region.loads {
            match (k, &locals[s]) {
                (K::Array(_), Value::Array(a)) => buf[s] = Rc::into_raw(a.clone()) as i64,
                (K::String, Value::String(st)) => buf[s] = Rc::into_raw(st.clone()) as i64,
                _ => {}
            }
        }
        let mut ctx = [0i64; 2];
        let cp = ctx.as_mut_ptr() as i64;
        let run = unsafe { std::mem::transmute::<usize, extern "C" fn(i64, i64, i64) -> i64>(region.ptr) };
        run(cp, 0, buf.as_mut_ptr() as i64);
        // no replay follows a block: the writes made before a failure stay, since the program stops there
        UNDO.with(|u| u.borrow_mut().finish(true));
        // the prints made before a failure happened, in the VM too: they stay
        settle_output(true);
        if ctx[0] != 0 {
            return BlockOutcome::Failed;
        }
        for &(s, k) in &region.stores {
            let tag = match k {
                K::Float => 2,
                K::Bool => 1,
                K::String => 3,
                _ => 0,
            };
            locals[s] = value_of(buf[s], tag);
        }
        BlockOutcome::Ran
    }

    fn has(&self, func_index: usize) -> bool {
        matches!(self.entries.get(func_index), Some(Some(_)))
    }

    fn try_loop(&self, chunk: usize, entry: usize, exit: usize, locals: &mut [Value]) -> LoopOutcome {
        let key = (chunk, exit);
        let known = self.regions.borrow().get(&key).cloned();
        let region = match known {
            Some(Some(r)) => r,
            Some(None) => return LoopOutcome::Never,
            None => {
                let n = {
                    let mut counts = self.counts.borrow_mut();
                    let c = counts.entry(key).or_insert(0);
                    *c += 1;
                    *c
                };
                if n < HOT_LOOP_ENTRIES {
                    return LoopOutcome::Skipped;
                }
                let compiled = match self.compile_region(chunk, exit, locals) {
                    Ok(r) => Some(Rc::new(r)),
                    Err(why) => {
                        self.refused.borrow_mut().insert(key, why);
                        None
                    }
                };
                self.regions.borrow_mut().insert(key, compiled.clone());
                match compiled {
                    Some(r) => r,
                    None => return LoopOutcome::Never,
                }
            }
        };
        // the VM is at an instruction of the loop (not before its head), and the locals have the kinds it was built for
        if entry < region.head || entry >= exit || locals.len() != region.nlocals {
            return LoopOutcome::Skipped;
        }
        let mut buf = vec![0i64; region.nlocals];
        for &(s, k) in &region.loads {
            buf[s] = match (k, &locals[s]) {
                (K::Int, Value::Int(n)) => *n,
                (K::Float, Value::Float(f)) => f.to_bits() as i64,
                (K::Bool, Value::Bool(b)) => *b as i64,
                (K::Array(_), Value::Array(_)) => 0,
                _ => return LoopOutcome::Skipped,
            };
        }
        // every check passed: now the region takes its own reference to each array it reads
        for &(s, k) in &region.loads {
            if let (K::Array(_), Value::Array(a)) = (k, &locals[s]) {
                buf[s] = Rc::into_raw(a.clone()) as i64;
            }
        }
        let mut ctx = [0i64; 2];
        let cp = ctx.as_mut_ptr() as i64;
        let run = unsafe { std::mem::transmute::<usize, extern "C" fn(i64, i64, i64) -> i64>(region.ptr) };
        run(cp, 0, buf.as_mut_ptr() as i64);
        if ctx[0] != 0 {
            // undo the array writes made so far: the VM runs this loop from its start. and keeps doing so
            UNDO.with(|u| u.borrow_mut().finish(false));
            settle_output(false);
            self.regions.borrow_mut().insert(key, None);
            return LoopOutcome::Never;
        }
        UNDO.with(|u| u.borrow_mut().finish(true));
        settle_output(true);
        for &(s, k) in &region.stores {
            locals[s] = match k {
                K::Float => Value::Float(f64::from_bits(buf[s] as u64)),
                K::Bool => Value::Bool(buf[s] != 0),
                _ => Value::Int(buf[s]),
            };
        }
        LoopOutcome::Ran
    }

    fn call(&self, func_index: usize, args: &[i64]) -> Option<i64> {
        let (ptr, arity) = (*self.entries.get(func_index)?)?;
        let owned = &self.arg_owned[func_index];
        if args.len() != arity || self.disabled[func_index].get() {
            // we were handed references the native code will never see. give them back.
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
        // every native function takes (ctx, depth, args...). the top call starts at depth 0
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
        // args included. tidy.
        if ctx[0] != 0 {
            // the VM reruns this call, so the stores made before the failure must go
            UNDO.with(|u| u.borrow_mut().finish(false));
            settle_output(false);
            self.disabled[func_index].set(true);
            None
        } else {
            UNDO.with(|u| u.borrow_mut().finish(true));
            settle_output(true);
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
            OpCode::Return | OpCode::Throw => {
                set.insert(i + 1);
            }
            _ => {}
        }
    }
    set.remove(&0);
    set.retain(|&i| i < chunk.code.len());
    set
}

/// whether a value of this kind is reference counted. heap stuff.
fn is_heap(k: K) -> bool {
    matches!(k, K::String | K::Array(_) | K::TArray(_) | K::Struct(_))
}

/// why native code turned a function or a loop down. `--jit-report` prints these
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// a param or the return type is not int, float, bool, string, or an array of those
    Signature,
    /// more params than a native call takes
    Arity,
    /// the code doesn't end with a return or a jump
    NoEnd,
    /// a jump lands outside the code
    BadJump,
    /// a value is left on the stack where control can also arrive from elsewhere
    StackAcrossJump,
    /// an instruction native code doesn't have
    Opcode,
    /// a builtin native code doesn't have (the name)
    Builtin(String),
    /// a constant native code doesn't hold
    Constant,
    /// a local changes kind, or is read before it has one
    LocalKind,
    /// an operand has the wrong kind for its instruction
    Operands,
    /// builds a struct with a field native code can't hold (an array, or another struct)
    StructField,
    /// an instruction needs more values than the stack holds
    Stack,
    /// calls a function that is not native (the function's index)
    Callee(usize),
    /// returns a value of the wrong kind
    ReturnKind,
    /// an array whose element kind native code can't hold
    ArrayElem,
    /// a string or array local is assigned inside a loop
    HeapAssigned,
    /// a jump leaves the loop by somewhere other than its exit
    JumpOutOfLoop,
    /// the loop doesn't end with its backward jump
    NotALoop,
    /// touches a `compile VM` block, which always runs in the VM
    VmBlock,
    /// contains a `compile JIT` block: the block runs as native code of its own
    JitBlock,
    /// jumps out of a `compile JIT` block (a break or continue of an outer loop, or a jump past its end)
    JumpOutOfBlock,
    /// returns from inside a `compile JIT` block: a block is not a function
    ReturnInBlock,
    /// native code generation failed
    Internal,
}

/// one refusal: its reason, and the instruction that caused it (`pc` in the checked chunk)
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub pc: Option<usize>,
    pub op: String,
    pub reason: Reason,
}

fn refusal(reason: Reason) -> Refusal {
    Refusal { pc: None, op: String::new(), reason }
}

fn at(pc: usize, op: &OpCode, reason: Reason) -> Refusal {
    // the variant's name: everything before its first payload
    let name: String = format!("{op:?}").chars().take_while(|c| c.is_alphanumeric()).collect();
    Refusal { pc: Some(pc), op: name, reason }
}

/// type-checks the unoptimized bytecode of one function against the signatures
/// of its callees, and returns the kind of every local slot. returns the refusal
/// for anything the native backend doesn't support: unsupported opcodes or
/// constants, operands of the wrong kind, a local read before its kind is known,
/// a local that changes kind, values left on the stack across a jump, or a
/// return of the wrong kind. the bouncer, basically.
fn check(chunk: &Chunk, own: &Sig, sigs: &[Option<Sig>], structs: &[Rc<spg_vm::value::StructDef>], typed: bool) -> Result<Vec<K>, Refusal> {
    let init: Vec<Option<K>> = own.0.iter().map(|k| Some(*k)).collect();
    check_init(chunk, &init, own.1, sigs, structs, typed)
}

fn pop(st: &mut Vec<K>, want: K) -> Result<(), Reason> {
    if st.pop().ok_or(Reason::Stack)? == want {
        Ok(())
    } else {
        Err(Reason::Operands)
    }
}

fn bin(st: &mut Vec<K>, a: K, b: K, out: K) -> Result<(), Reason> {
    pop(st, b)?;
    pop(st, a)?;
    st.push(out);
    Ok(())
}

fn un(st: &mut Vec<K>, a: K, out: K) -> Result<(), Reason> {
    pop(st, a)?;
    st.push(out);
    Ok(())
}

/// one instruction of the checker: its effect on the kinds of the stack and the locals
#[allow(clippy::too_many_arguments)]
fn step(chunk: &Chunk, sigs: &[Option<Sig>], structs: &[Rc<spg_vm::value::StructDef>], typed: bool, ret: K, op: &OpCode, st: &mut Vec<K>, slots: &mut [Option<K>]) -> Result<(), Reason> {
    match op {
        OpCode::Const(k) => st.push(match chunk.constants.get(*k).ok_or(Reason::Constant)? {
            Value::Int(_) => K::Int,
            Value::Float(_) => K::Float,
            Value::Bool(_) => K::Bool,
            Value::String(_) => K::String,
            Value::Enum(def, _) => K::Enum(*def as usize),
            _ => return Err(Reason::Constant),
        }),
        OpCode::GetLocal(s) => {
            let k = (*slots.get(*s).ok_or(Reason::LocalKind)?).ok_or(Reason::LocalKind)?;
            st.push(k);
        }
        OpCode::SetLocal(s) => {
            let k = *st.last().ok_or(Reason::Stack)?;
            match slots.get(*s).ok_or(Reason::LocalKind)? {
                Some(old) if *old != k => return Err(Reason::LocalKind),
                _ => slots[*s] = Some(k),
            }
        }
        OpCode::Pop => {
            st.pop().ok_or(Reason::Stack)?;
        }
        // the result of a function that returns nothing
        OpCode::PushVoid => st.push(K::Void),
        // native code never raises. it fails the whole call, and the VM reruns it and raises the exact
        // error with its trace. so any kind can be thrown, it is only ever released
        OpCode::Throw => {
            st.pop().ok_or(Reason::Stack)?;
            st.clear();
        }
        OpCode::AddInt | OpCode::SubInt | OpCode::MulInt | OpCode::DivInt | OpCode::ModInt => {
            bin(st, K::Int, K::Int, K::Int)?
        }
        OpCode::NegInt => un(st, K::Int, K::Int)?,
        OpCode::AddFloat | OpCode::SubFloat | OpCode::MulFloat | OpCode::DivFloat | OpCode::ModFloat
        | OpCode::PowFloat => bin(st, K::Float, K::Float, K::Float)?,
        OpCode::NegFloat => un(st, K::Float, K::Float)?,
        OpCode::ConcatString => bin(st, K::String, K::String, K::String)?,
        OpCode::EqString | OpCode::NotEqString => bin(st, K::String, K::String, K::Bool)?,
        OpCode::LtInt | OpCode::GtInt | OpCode::LtEqInt | OpCode::GtEqInt => {
            bin(st, K::Int, K::Int, K::Bool)?
        }
        // the compiler emits EqInt for `==` on enums (the VM reads a variant as its index).
        // so an enum compares as an int here, but only with an enum of the same type.
        OpCode::EqInt | OpCode::NotEqInt => {
            let y = st.pop().ok_or(Reason::Stack)?;
            let x = st.pop().ok_or(Reason::Stack)?;
            match (x, y) {
                (K::Int, K::Int) => st.push(K::Bool),
                (K::Enum(a), K::Enum(b)) if a == b => st.push(K::Bool),
                _ => return Err(Reason::Operands),
            }
        }
        OpCode::LtFloat | OpCode::GtFloat | OpCode::LtEqFloat | OpCode::GtEqFloat | OpCode::EqFloat
        | OpCode::NotEqFloat => bin(st, K::Float, K::Float, K::Bool)?,
        OpCode::EqBool | OpCode::NotEqBool => bin(st, K::Bool, K::Bool, K::Bool)?,
        OpCode::Not => un(st, K::Bool, K::Bool)?,
        // deep equality is only native for enums: a variant index compares as an int.
        // arrays, maps and structs compare by content, which native code can't do.
        OpCode::EqValue | OpCode::NotEqValue => {
            let y = st.pop().ok_or(Reason::Stack)?;
            let x = st.pop().ok_or(Reason::Stack)?;
            match (x, y) {
                (K::Enum(a), K::Enum(b)) if a == b => st.push(K::Bool),
                _ => return Err(Reason::Operands),
            }
        }
        OpCode::IndexGet => {
            // stack: array, index
            pop(st, K::Int)?;
            match st.pop().ok_or(Reason::Stack)? {
                K::Array(e) | K::TArray(e) => st.push(e.value_kind()),
                _ => return Err(Reason::Operands),
            }
        }
        OpCode::IndexSet => {
            // stack: array, index, value. the value must have the element's kind
            let v = st.pop().ok_or(Reason::Stack)?;
            pop(st, K::Int)?;
            match st.pop().ok_or(Reason::Stack)? {
                K::Array(e) | K::TArray(e) if e.value_kind() == v => {}
                _ => return Err(Reason::Operands),
            }
        }
        OpCode::Dup2 => {
            let n = st.len();
            if n < 2 {
                return Err(Reason::Stack);
            }
            let (a, c) = (st[n - 2], st[n - 1]);
            st.push(a);
            st.push(c);
        }
        OpCode::FieldGet { field_index } => {
            // stack: struct
            let K::Struct(d) = st.pop().ok_or(Reason::Stack)? else { return Err(Reason::Operands) };
            let fk = (*structs.get(d).ok_or(Reason::Operands)?.field_kinds.get(*field_index).ok_or(Reason::Operands)?)
                .ok_or(Reason::Operands)?;
            st.push(fk);
        }
        OpCode::FieldSet { field_index } => {
            // stack: struct, value
            let v = st.pop().ok_or(Reason::Stack)?;
            let K::Struct(d) = st.pop().ok_or(Reason::Stack)? else { return Err(Reason::Operands) };
            let fk = (*structs.get(d).ok_or(Reason::Operands)?.field_kinds.get(*field_index).ok_or(Reason::Operands)?)
                .ok_or(Reason::Operands)?;
            if fk != v {
                return Err(Reason::Operands);
            }
        }
        OpCode::Jump(_) => {
            if !st.is_empty() {
                return Err(Reason::StackAcrossJump);
            }
        }
        OpCode::JumpIfFalse(_) => {
            pop(st, K::Bool)?;
            if !st.is_empty() {
                return Err(Reason::StackAcrossJump);
            }
        }
        OpCode::Call { func_index, argc } => {
            let (ps, r) = sigs
                .get(*func_index)
                .and_then(|s| s.as_ref())
                .ok_or(Reason::Callee(*func_index))?;
            if *argc != ps.len() || st.len() < *argc {
                return Err(Reason::Operands);
            }
            let base = st.len() - *argc;
            if st[base..] != ps[..] {
                return Err(Reason::Operands);
            }
            st.truncate(base);
            st.push(*r);
        }
        OpCode::CallBuiltin { id: BuiltinId::ArrayPush, argc: 2 } => {
            // stack: array, item. pushes a placeholder for the Void result, which the Pop right after drops
            let item = st.pop().ok_or(Reason::Stack)?;
            match st.pop().ok_or(Reason::Stack)? {
                K::Array(e) | K::TArray(e) if elem_kind(item) == Some(e) => st.push(K::Int),
                _ => return Err(Reason::Operands),
            }
        }
        OpCode::CallBuiltin { id: BuiltinId::Print, argc } => {
            if *argc > 31 || st.len() < *argc {
                return Err(Reason::Operands);
            }
            let base = st.len() - *argc;
            if st[base..].iter().any(|k| !matches!(k, K::Int | K::Float | K::Bool | K::String)) {
                return Err(Reason::Builtin("print of an array or struct".to_string()));
            }
            st.truncate(base);
            st.push(K::Int);
        }
        OpCode::CallBuiltin { id, argc } => {
            // only pure builtins. everything else stays in the VM. the rest is not our problem.
            if let BuiltinId::ArrayLenBuiltin = id {
                if *argc != 1 || !matches!(st.last(), Some(K::Array(_) | K::TArray(_))) {
                    return Err(Reason::Operands);
                }
                st.pop();
                st.push(K::Int);
                return Ok(());
            }
            let (args, out): (&[K], K) = match id {
                BuiltinId::Len => (&[K::String], K::Int),
                BuiltinId::IntToString => (&[K::Int], K::String),
                BuiltinId::FloatToString => (&[K::Float], K::String),
                BuiltinId::BoolToString => (&[K::Bool], K::String),
                // exact conversions: `as` in Rust, which saturates and maps NaN to 0. math is weird.
                BuiltinId::IntToFloat => (&[K::Int], K::Float),
                BuiltinId::FloatToInt => (&[K::Float], K::Int),
                _ => return Err(Reason::Builtin(format!("{id:?}"))),
            };
            if *argc != args.len() || st.len() < *argc || st[st.len() - *argc..] != *args {
                return Err(Reason::Operands);
            }
            st.truncate(st.len() - *argc);
            st.push(out);
        }
        OpCode::Return => {
            if st.pop().ok_or(Reason::Stack)? != ret {
                return Err(Reason::ReturnKind);
            }
            st.clear();
        }
        OpCode::MakeEmptyArray(tag) => {
            let e = match tag {
                0 => ElemKind::Int,
                1 => ElemKind::Bool,
                2 => ElemKind::Float,
                3 => ElemKind::String,
                _ => return Err(Reason::ArrayElem),
            };
            if typed && matches!(e, ElemKind::String) {
                return Err(Reason::ArrayElem);
            }
            st.push(array_kind(e, typed));
        }
        OpCode::MakeArray(n) => {
            // every item is a scalar of one kind, so the array's element kind is that kind
            let mut kind: Option<K> = None;
            for _ in 0..*n {
                let k = st.pop().ok_or(Reason::Stack)?;
                elem_kind(k).ok_or(Reason::ArrayElem)?;
                if kind.is_some_and(|prev| prev != k) {
                    return Err(Reason::Operands);
                }
                kind = Some(k);
            }
            let e = elem_kind(kind.ok_or(Reason::ArrayElem)?).ok_or(Reason::ArrayElem)?;
            if typed && matches!(e, ElemKind::String) {
                return Err(Reason::ArrayElem);
            }
            st.push(array_kind(e, typed));
        }
        OpCode::MakeStruct { def_index, field_count } => {
            // stack: the fields, in declaration order. every field must be one native code can hold,
            // and each value must be of its field's kind
            let def = structs.get(*def_index).ok_or(Reason::Operands)?;
            if def.field_kinds.len() != *field_count || st.len() < *field_count {
                return Err(Reason::Operands);
            }
            if def.field_kinds.iter().any(|k| k.is_none()) {
                return Err(Reason::StructField);
            }
            let base = st.len() - *field_count;
            if st[base..].iter().zip(&def.field_kinds).any(|(k, want)| Some(*k) != *want) {
                return Err(Reason::Operands);
            }
            st.truncate(base);
            st.push(K::Struct(*def_index));
        }
        OpCode::JitBlock { .. } => return Err(Reason::JitBlock),
        // globals, maps, the rest of the builtins: still in the VM
        _ => return Err(Reason::Opcode),
    }
    Ok(())
}

/// the checker behind `check` and loop regions. `init` gives the kinds of the slots that hold
/// a value on entry, `ret` the kind a `Return` hands back
fn check_init(chunk: &Chunk, init: &[Option<K>], ret: K, sigs: &[Option<Sig>], structs: &[Rc<spg_vm::value::StructDef>], typed: bool) -> Result<Vec<K>, Refusal> {
    let code = &chunk.code;
    if code.is_empty() || !matches!(code.last(), Some(OpCode::Return) | Some(OpCode::Jump(_)) | Some(OpCode::Throw)) {
        return Err(refusal(Reason::NoEnd));
    }
    // a jump may land one past the end: that's the "after the function" label. such a jump
    // only sits in dead code (the `else` of an if whose branches both return). if it's
    // ever taken, the native call fails and the VM runs the function. anything further out gets refused.
    for (i, op) in code.iter().enumerate() {
        match op {
            OpCode::Jump(t) if *t > code.len() => return Err(at(i, op, Reason::BadJump)),
            OpCode::JumpIfFalse(t) if *t > code.len() || i + 1 >= code.len() => {
                return Err(at(i, op, Reason::BadJump))
            }
            _ => {}
        }
    }
    let leaders = leaders(chunk);
    let mut slots: Vec<Option<K>> = vec![None; chunk.num_locals];
    for (i, k) in init.iter().enumerate() {
        if let Some(k) = k {
            *slots.get_mut(i).ok_or(refusal(Reason::LocalKind))? = Some(*k);
        }
    }
    let mut st: Vec<K> = Vec::new();
    for (i, op) in code.iter().enumerate() {
        if leaders.contains(&i) && !st.is_empty() {
            return Err(at(i, op, Reason::StackAcrossJump));
        }
        step(chunk, sigs, structs, typed, ret, op, &mut st, &mut slots).map_err(|r| at(i, op, r))?;
    }
    // a local that's never written is never read either. give it a kind anyway.
    Ok(slots.into_iter().map(|k| k.unwrap_or(K::Int)).collect())
}

/// which functions get native code: a fixpoint over the call graph, so a
/// function that calls a non-native function isn't native either. guilt by association.
/// also returns, for every function that isn't native, the refusal that said so.
fn eligible(program: &Program) -> (Vec<bool>, Vec<Option<Vec<K>>>, Vec<bool>, Vec<Option<Refusal>>) {
    let n = program.functions.len();
    let mut reasons: Vec<Option<Refusal>> = vec![None; n];
    let mut elig = vec![false; n];
    for (i, c) in program.functions.iter().enumerate() {
        if c.native_sig.is_none() {
            reasons[i] = Some(refusal(Reason::Signature));
        } else if c.arity > MAX_ARITY {
            reasons[i] = Some(refusal(Reason::Arity));
        } else if let Some(&(start, _)) = c.vm_only.first() {
            reasons[i] = Some(Refusal { pc: Some(start), op: String::new(), reason: Reason::VmBlock });
        } else {
            elig[i] = true;
        }
    }
    loop {
        let sigs: Vec<Option<Sig>> = (0..n)
            .map(|i| if elig[i] { program.functions[i].native_sig.clone() } else { None })
            .collect();
        let mut changed = false;
        let mut slots = vec![None; n];
        let mut typed = vec![false; n];
        for i in 0..n {
            if !elig[i] {
                continue;
            }
            let own = sigs[i].clone().unwrap();
            // typed mode first: arrays that never leave the function become native-only buffers
            let found = match check(&program.functions[i], &own, &sigs, &program.struct_defs, true) {
                Ok(s) => Some((s, true)),
                Err(_) => match check(&program.functions[i], &own, &sigs, &program.struct_defs, false) {
                    Ok(s) => Some((s, false)),
                    Err(why) => {
                        reasons[i] = Some(why);
                        None
                    }
                },
            };
            match found {
                Some((s, t)) => {
                    slots[i] = Some(s);
                    typed[i] = t;
                }
                None => {
                    elig[i] = false;
                    changed = true;
                }
            }
        }
        if !changed {
            return (elig, slots, typed, reasons);
        }
    }
}

/// the runtime helpers, declared inside one function
fn helpers_in(module: &mut JITModule, decls: &[(&'static str, FuncId)], func: &mut cranelift_codegen::ir::Function) -> Helpers {
    let id_of = |name: &str| decls.iter().find(|(n, _)| *n == name).unwrap().1;
    let mut declare = |name: &str| module.declare_func_in_func(id_of(name), func);
    Helpers {
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
        print: declare("spg_print"),
        arr_get_int: declare("spg_arr_get_int"),
        arr_get_bool: declare("spg_arr_get_bool"),
        arr_get_float: declare("spg_arr_get_float"),
        arr_get_str: declare("spg_arr_get_str"),
        arr_set: declare("spg_arr_set"),
        struct_set: declare("spg_struct_set"),
        struct_get: declare("spg_struct_get"),
        struct_retain: declare("spg_struct_retain"),
        struct_release: declare("spg_struct_release"),
        struct_new: declare("spg_struct_new"),
        arr_new: declare("spg_arr_new"),
        arr_add: declare("spg_arr_add"),
        arr_push: declare("spg_arr_push"),
        arr_put: declare("spg_arr_put"),
        tarr_new: declare("spg_tarr_new"),
        tarr_retain: declare("spg_tarr_retain"),
        tarr_release: declare("spg_tarr_release"),
        tarr_len: declare("spg_tarr_len"),
        tarr_get: declare("spg_tarr_get"),
        tarr_set: declare("spg_tarr_set"),
        tarr_push: declare("spg_tarr_push"),
    }
}

/// references to the runtime helpers, as seen from one native function
#[derive(Clone, Copy)]
struct Helpers {
    print: FuncRef,
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
    struct_new: FuncRef,
    arr_new: FuncRef,
    arr_add: FuncRef,
    arr_push: FuncRef,
    arr_put: FuncRef,
    tarr_new: FuncRef,
    tarr_retain: FuncRef,
    tarr_release: FuncRef,
    tarr_len: FuncRef,
    tarr_get: FuncRef,
    tarr_set: FuncRef,
    tarr_push: FuncRef,
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
            K::TArray(_) => {
                b.ins().call(self.tarr_retain, &[v]);
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
            K::TArray(_) => {
                b.ins().call(self.tarr_release, &[v]);
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
/// `temps` get released on the way out.
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
/// the flag, return 0. bye.
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

/// a stack value as the bits and tag the array and struct helpers take (0 int, 1 bool, 2 float, 3 string)
fn elem_bits(b: &mut FunctionBuilder, v: CV, k: K) -> (CV, CV) {
    let (bits, tag) = match k {
        K::Float => (b.ins().bitcast(I64, MemFlags::new(), v), 2),
        K::Bool => (v, 1),
        K::String => (v, 3),
        _ => (v, 0),
    };
    (bits, b.ins().iconst(I64, tag))
}

/// the element kind of an array of this runtime value (None: empty, or not a scalar element)
fn elem_of(items: &[Value]) -> Option<ElemKind> {
    match items.first()? {
        Value::Int(_) => Some(ElemKind::Int),
        Value::Float(_) => Some(ElemKind::Float),
        Value::Bool(_) => Some(ElemKind::Bool),
        Value::String(_) => Some(ElemKind::String),
        _ => None,
    }
}

/// the element the native kinds stand for, when it's an array element
/// the kind of a new array: typed (native-only, plain bits) or the VM's representation
fn array_kind(e: ElemKind, typed: bool) -> K {
    if typed {
        K::TArray(e)
    } else {
        K::Array(e)
    }
}

fn elem_kind(k: K) -> Option<ElemKind> {
    match k {
        K::Int => Some(ElemKind::Int),
        K::Float => Some(ElemKind::Float),
        K::Bool => Some(ElemKind::Bool),
        K::String => Some(ElemKind::String),
        _ => None,
    }
}

/// the element tag the struct helpers take: 0 int, 1 bool, 2 float, 3 string
fn struct_tag(k: K) -> i64 {
    match k {
        K::Int => 0,
        K::Bool => 1,
        K::Float => 2,
        K::String => 3,
        // enum fields: tag 4 + the enum's def index, so value_of can rebuild Value::Enum
        K::Enum(d) => 4 + d as i64,
        _ => unreachable!("check() admits only scalar, string and enum fields"),
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
    // loop regions: (slots to load from the locals array on entry, slots to store back on exit)
    region: Option<(&[bool], &[bool])>,
    typed: bool,
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
    // params arrive owned. the other locals start empty (null / zero)
    for (i, (v, k)) in vars.iter().zip(slot_kinds).enumerate() {
        let raw = if let Some((loads, _)) = region {
            if loads.get(i).copied().unwrap_or(false) {
                b.ins().load(I64, MemFlags::trusted(), params[2], (8 * i) as i32)
            } else {
                b.ins().iconst(I64, 0)
            }
        } else if i < chunk.arity {
            params[i + 2]
        } else {
            b.ins().iconst(I64, 0)
        };
        let val = if *k == K::Float { b.ins().bitcast(F64, MemFlags::new(), raw) } else { raw };
        b.def_var(*v, val);
    }

    // depth limit, like the VM's MAX_CALL_DEPTH. deeper calls fall back to the VM.
    // the depth travels in an arg register, not in memory. fast.
    let limit = b.ins().iconst(I64, call_depth_limit() as i64);
    let too_deep = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, depth, limit);
    let body = b.create_block();
    b.ins().brif(too_deep, fail, &[], body, &[]);
    b.switch_to_block(body);
    let deeper = b.ins().iadd_imm(depth, 1);

    // one Cranelift block per leader. instruction 0 gets its own block, because a
    // loop may jump back to it. loops, always loops.
    let start = b.create_block();
    b.ins().jump(start, &[]);
    b.switch_to_block(start);
    let leader_set = leaders(chunk);
    let mut blocks: HashMap<usize, Block> = HashMap::new();
    blocks.insert(0, start);
    for &i in &leader_set {
        blocks.insert(i, b.create_block());
    }
    // a loop region leaves through its exit: the one past the end, where the locals get stored back
    let exit_block = region.map(|_| {
        let eb = b.create_block();
        blocks.insert(chunk.code.len(), eb);
        eb
    });

    let mut stack: Vec<(CV, K)> = Vec::new();
    let mut terminated = false;
    // a fresh array is one this function made and nothing else holds: its slot only ever gets
    // a new literal. pushes to it need no undo entry, since a failed call throws it away.
    let fresh_slot = |s: usize| -> bool {
        // a slot that's never set here (a param, or an array a region got from the VM) is not fresh
        s >= chunk.arity
            && chunk.code.iter().any(|op| matches!(op, OpCode::SetLocal(t) if *t == s))
            && chunk.code.iter().enumerate().all(|(i, op)| match op {
                OpCode::SetLocal(t) if *t == s => {
                    i > 0 && matches!(chunk.code[i - 1], OpCode::MakeArray(_) | OpCode::MakeEmptyArray(_))
                }
                _ => true,
            })
    };
    // the values that hold a fresh array right now
    let mut fresh_vals: std::collections::HashSet<CV> = std::collections::HashSet::new();
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
                        // the constant lives on in the NativeTable. the stack gets its own reference
                        let p = b.ins().iconst(I64, Rc::as_ptr(rc) as usize as i64);
                        h.retain(b, K::String, p);
                        (p, K::String)
                    }
                    Value::Enum(def, variant) => (b.ins().iconst(I64, *variant as i64), K::Enum(*def as usize)),
                    _ => unreachable!("check() rejects other constants"),
                };
                stack.push(entry);
            }
            OpCode::GetLocal(s) => {
                let v = b.use_var(vars[*s]);
                let k = slot_kinds[*s];
                h.retain(b, k, v);
                if matches!(k, K::Array(_)) && fresh_slot(*s) {
                    fresh_vals.insert(v);
                }
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
            OpCode::PushVoid => stack.push((b.ins().iconst(I64, 0), K::Void)),
            OpCode::Throw => {
                // the thrown value is ours, and so is whatever else sits on the stack. release it all,
                // then fail the call: `fail` releases the heap locals and sets the flag
                let (v, k) = stack.pop().unwrap();
                h.release(b, k, v);
                for (t, k) in heap_temps(&stack) {
                    h.release(b, k, t);
                }
                stack.clear();
                b.ins().jump(fail, &[]);
                terminated = true;
            }
            OpCode::AddInt | OpCode::SubInt => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let (r, bad) = if matches!(op, OpCode::AddInt) {
                    // signed add overflows iff both operands share a sign the result doesn't have
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
                // widen to 128 bits. the product fits iff sign-extending its low half gives it back
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
                // by zero, and by -1 (where MIN / -1 overflows). let the VM decide
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
                // the helper eats both operands
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
            OpCode::IndexGet if matches!(stack.len().checked_sub(2).map(|i| stack[i].1), Some(K::TArray(_))) => {
                let (idx, _) = stack.pop().unwrap();
                let (arr, ak) = stack.pop().unwrap();
                let K::TArray(elem) = ak else { unreachable!("guarded above") };
                let len = b.ins().call(h.tarr_len, &[arr]);
                let len = b.inst_results(len)[0];
                let bad = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);
                let mut temps = heap_temps(&stack);
                temps.push((arr, ak));
                guard(b, h, bad, fail, temps);
                let got = b.ins().call(h.tarr_get, &[arr, idx]);
                let bits = b.inst_results(got)[0];
                h.release(b, ak, arr);
                let v = if matches!(elem, ElemKind::Float) { b.ins().bitcast(F64, MemFlags::new(), bits) } else { bits };
                stack.push((v, elem.value_kind()));
            }
            OpCode::IndexGet => {
                let (idx, _) = stack.pop().unwrap();
                let (arr, ak) = stack.pop().unwrap();
                let elem = match ak {
                    K::Array(e) => e,
                    _ => unreachable!("check() ensures an array"),
                };
                // unsigned compare: a negative index is out of range too. sneaky, good.
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
                // the element was read by borrow (or cloned for strings). the array ref is spent
                h.release(b, ak, arr);
                stack.push((val, k));
            }
            OpCode::IndexSet if matches!(stack.len().checked_sub(3).map(|i| stack[i].1), Some(K::TArray(_))) => {
                let (val, _) = stack.pop().unwrap();
                let (idx, _) = stack.pop().unwrap();
                let (arr, ak) = stack.pop().unwrap();
                let K::TArray(elem) = ak else { unreachable!("guarded above") };
                let len = b.ins().call(h.tarr_len, &[arr]);
                let len = b.inst_results(len)[0];
                let bad = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, len);
                let mut temps = heap_temps(&stack);
                temps.push((arr, ak));
                guard(b, h, bad, fail, temps);
                let bits = if matches!(elem, ElemKind::Float) { b.ins().bitcast(I64, MemFlags::new(), val) } else { val };
                b.ins().call(h.tarr_set, &[arr, idx, bits]);
                h.release(b, ak, arr);
            }
            OpCode::IndexSet => {
                // stack: array, index, value. a string value's ref moves into the array
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
                let inst = b.ins().call(if fresh_vals.contains(&arr) { h.arr_put } else { h.arr_set }, &[arr, idx, bits, tag]);
                let status = b.inst_results(inst)[0];
                // the store ate a string value either way. only a borrow clash lands here
                let mut temps = heap_temps(&stack);
                temps.push((arr, ak));
                guard(b, h, status, fail, temps);
                h.release(b, ak, arr);
            }
            OpCode::Dup2 => {
                // the top two values again. heap ones get their own ref
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
                // stack: struct. the struct ref is ours, released after the read.
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
                // stack: struct, value. a string value's ref moves into the struct
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
                // the store ate a string value either way. only a borrow clash lands here
                let mut temps = heap_temps(&stack);
                temps.push((s, sk));
                guard(b, h, status, fail, temps);
                h.release(b, sk, s);
            }
            OpCode::MakeStruct { def_index, field_count } => {
                // stack: the fields, in order. their values go to a stack buffer and the helper builds
                // the struct. a string field's ref moves into it, so nothing is left to release
                let at = stack.len() - field_count;
                let fields: Vec<(CV, K)> = stack.drain(at..).collect();
                let slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (8 * fields.len().max(1)) as u32));
                for (i, &(v, _)) in fields.iter().enumerate() {
                    let ty = b.func.dfg.value_type(v);
                    let bits = if ty == F64 {
                        b.ins().bitcast(I64, MemFlags::new(), v)
                    } else if ty == I64 {
                        v
                    } else {
                        b.ins().uextend(I64, v)
                    };
                    b.ins().stack_store(bits, slot, Offset32::new((8 * i) as i32));
                }
                let buf = b.ins().stack_addr(I64, slot, Offset32::new(0));
                let def = b.ins().iconst(I64, Rc::as_ptr(&structs[*def_index]) as usize as i64);
                let n = b.ins().iconst(I64, fields.len() as i64);
                let inst = b.ins().call(h.struct_new, &[def, buf, n]);
                stack.push((b.inst_results(inst)[0], K::Struct(*def_index)));
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
            OpCode::EqValue | OpCode::NotEqValue => {
                // enums only (the checker refused everything else): same type, so compare the variant index
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                let cc = if matches!(op, OpCode::EqValue) { IntCC::Equal } else { IntCC::NotEqual };
                let c = b.ins().icmp(cc, x, y);
                stack.push((b.ins().uextend(I64, c), K::Bool));
            }
            OpCode::LtFloat | OpCode::GtFloat | OpCode::LtEqFloat | OpCode::GtEqFloat | OpCode::EqFloat
            | OpCode::NotEqFloat => {
                let (y, _) = stack.pop().unwrap();
                let (x, _) = stack.pop().unwrap();
                // IEEE comparisons, like the VM: NaN is unequal to everything, so `!=`
                // is true for it. FloatCC::NotEqual is "unordered or not equal". nan is weird.
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
                // callees take every arg as its 64-bit pattern. the args get eaten
                let mut call_args = vec![ctx, deeper];
                for ((v, _), k) in args.iter().zip(callee_ps) {
                    call_args.push(if k == K::Float { b.ins().bitcast(I64, MemFlags::new(), *v) } else { *v });
                }
                let fref = refs[func_index];
                let inst = b.ins().call(fref, &call_args);
                let raw = b.inst_results(inst)[0];
                // a callee that failed left its flag set. its result is meaningless. ignore it.
                let failed = b.ins().load(I64, MemFlags::trusted(), ctx, CTX_FAILED);
                guard(b, h, failed, fail, heap_temps(&stack));
                let val = if callee_r == K::Float { b.ins().bitcast(F64, MemFlags::new(), raw) } else { raw };
                stack.push((val, callee_r));
            }
            OpCode::CallBuiltin { id, argc } => {
                let at = stack.len() - argc;
                let args: Vec<(CV, K)> = stack.drain(at..).collect();
                if matches!(id, BuiltinId::Print) {
                    // the values go to a stack buffer, their kinds packed two bits each; the helper queues the output
                    let slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (8 * args.len().max(1)) as u32));
                    let mut kinds: i64 = 0;
                    for (i, &(v, k)) in args.iter().enumerate() {
                        let ty = b.func.dfg.value_type(v);
                        let bits = if ty == F64 {
                            b.ins().bitcast(I64, MemFlags::new(), v)
                        } else if ty == I64 {
                            v
                        } else {
                            b.ins().uextend(I64, v)
                        };
                        b.ins().stack_store(bits, slot, Offset32::new((8 * i) as i32));
                        kinds |= match k {
                            K::Int => 0,
                            K::Float => 1,
                            K::Bool => 2,
                            _ => 3,
                        } << (2 * i);
                    }
                    let buf = b.ins().stack_addr(I64, slot, Offset32::new(0));
                    let n = b.ins().iconst(I64, args.len() as i64);
                    let kinds = b.ins().iconst(I64, kinds);
                    b.ins().call(h.print, &[buf, n, kinds]);
                    for &(v, k) in &args {
                        h.release(b, k, v);
                    }
                    let placeholder = b.ins().iconst(I64, 0);
                    stack.push((placeholder, K::Int));
                    continue;
                }
                if matches!(id, BuiltinId::ArrayLenBuiltin) && matches!(args[0].1, K::TArray(_)) {
                    let (arr, ak) = args[0];
                    let len = b.ins().call(h.tarr_len, &[arr]);
                    let len = b.inst_results(len)[0];
                    h.release(b, ak, arr);
                    stack.push((len, K::Int));
                    continue;
                }
                if matches!(id, BuiltinId::ArrayPush) && matches!(args[0].1, K::TArray(_)) {
                    let (arr, ak) = args[0];
                    let (item, ik) = args[1];
                    let bits = if matches!(ik, K::Float) { b.ins().bitcast(I64, MemFlags::new(), item) } else { item };
                    b.ins().call(h.tarr_push, &[arr, bits]);
                    h.release(b, ak, arr);
                    stack.push((b.ins().iconst(I64, 0), K::Int));
                    continue;
                }
                if matches!(id, BuiltinId::ArrayPush) {
                    let (arr, ak) = args[0];
                    let (item, ik) = args[1];
                    let (bits, tag) = elem_bits(b, item, ik);
                    // a fresh array (see fresh_slot) needs no undo entry; any other array is logged
                    let helper = if fresh_vals.contains(&arr) { h.arr_add } else { h.arr_push };
                    let failed = b.ins().call(helper, &[arr, bits, tag]);
                    let failed = b.inst_results(failed)[0];
                    // the builtin consumes the array reference (the item is consumed by the helper)
                    h.release(b, ak, arr);
                    guard(b, h, failed, fail, heap_temps(&stack));
                    stack.push((b.ins().iconst(I64, 0), K::Int));
                    continue;
                }
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
                        // ArrayLenBuiltin: count, then the array ref is spent
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
                // the value gets handed to the caller. everything else we still own goes
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
            OpCode::MakeEmptyArray(tag) if typed => {
                let arr = b.ins().call(h.tarr_new, &[]);
                let arr = b.inst_results(arr)[0];
                let e = match *tag {
                    0 => ElemKind::Int,
                    1 => ElemKind::Bool,
                    _ => ElemKind::Float,
                };
                stack.push((arr, K::TArray(e)));
            }
            OpCode::MakeArray(n) if typed => {
                let at = stack.len() - *n;
                let items: Vec<(CV, K)> = stack.drain(at..).collect();
                let arr = b.ins().call(h.tarr_new, &[]);
                let arr = b.inst_results(arr)[0];
                let elem = elem_kind(items[0].1).expect("check() ensures scalar items");
                for (v, k) in items {
                    let bits = if matches!(k, K::Float) { b.ins().bitcast(I64, MemFlags::new(), v) } else { v };
                    b.ins().call(h.tarr_push, &[arr, bits]);
                }
                stack.push((arr, K::TArray(elem)));
            }
            OpCode::MakeEmptyArray(tag) => {
                let arr = b.ins().call(h.arr_new, &[]);
                let arr = b.inst_results(arr)[0];
                let e = match *tag {
                    0 => ElemKind::Int,
                    1 => ElemKind::Bool,
                    2 => ElemKind::Float,
                    _ => ElemKind::String,
                };
                fresh_vals.insert(arr);
                stack.push((arr, K::Array(e)));
            }
            OpCode::MakeArray(n) => {
                let at = stack.len() - *n;
                let items: Vec<(CV, K)> = stack.drain(at..).collect();
                let arr = b.ins().call(h.arr_new, &[]);
                let arr = b.inst_results(arr)[0];
                let elem = elem_kind(items[0].1).expect("check() ensures scalar items");
                fresh_vals.insert(arr);
                // items go in bottom first, and each item's reference moves into the array
                for (v, k) in items {
                    let (bits, tag) = elem_bits(b, v, k);
                    b.ins().call(h.arr_add, &[arr, bits, tag]);
                }
                stack.push((arr, K::Array(elem)));
            }
            _ => unreachable!("check() rejects unsupported opcodes"),
        }
    }
    if let (Some((_, stores)), Some(exit_block)) = (region, exit_block) {
        b.switch_to_block(exit_block);
        for (i, store) in stores.iter().enumerate() {
            if *store {
                let v = b.use_var(vars[i]);
                let raw = if slot_kinds[i] == K::Float { b.ins().bitcast(I64, MemFlags::new(), v) } else { v };
                b.ins().store(MemFlags::trusted(), raw, params[2], (8 * i) as i32);
            }
        }
        for (var, k) in &heap_vars {
            // a string handed back to its local is the VM's now: only the rest is released here
            if stores.get(var.as_u32() as usize).copied().unwrap_or(false) {
                continue;
            }
            let x = b.use_var(*var);
            h.release(b, *k, x);
        }
        let zero = b.ins().iconst(I64, 0);
        b.ins().return_(&[zero]);
    }
    fail_block(b, h, ctx, fail, &heap_vars);
    b.seal_all_blocks();
}

/// whether two lists of struct types are the same types, in the same order
fn same_struct_defs(a: &[Rc<spg_vm::value::StructDef>], b: &[Rc<spg_vm::value::StructDef>]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| x.name == y.name && x.field_names == y.field_names && x.field_kinds == y.field_kinds)
}

/// compiles every eligible function of `program`. errors only if Cranelift
/// refuses the code. the VM then keeps running everything.
///
/// `vm_structs` are the struct types of the program the VM runs (the optimized copy, while
/// `program` is the plain one). native code builds structs of its own, and the VM tells a
/// struct's type by identity, so those structs must carry the VM's `StructDef`s. both copies
/// come from the same source, so the types agree. if they ever don't, native code stays off.
pub fn build(program: &Program, vm_structs: &[Rc<spg_vm::value::StructDef>]) -> Result<NativeTable, String> {
    if !same_struct_defs(&program.struct_defs, vm_structs) {
        return Err("the plain and the optimized program disagree on their struct types".to_string());
    }
    let unified;
    let program: &Program = if program.struct_defs.iter().zip(vm_structs).all(|(a, b)| Rc::ptr_eq(a, b)) {
        program
    } else {
        let mut p = program.clone();
        p.struct_defs = vm_structs.to_vec();
        unified = p;
        &unified
    };
    let (elig, slots, typed, reasons) = eligible(program);
    let sigs: Vec<Option<Sig>> = (0..program.functions.len())
        .map(|i| if elig[i] { program.functions[i].native_sig.clone() } else { None })
        .collect();

    // string constants get baked into the code by address. keep them alive
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
    // the platform's C calling convention: SysV on Linux, Windows x64 on Windows. must match
    // the Rust `extern "C"` helpers and the `extern "C"` entry point we call native through.
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
        ("spg_print", spg_print as *const u8),
        ("spg_arr_get_int", spg_arr_get_int as *const u8),
        ("spg_arr_get_bool", spg_arr_get_bool as *const u8),
        ("spg_arr_get_float", spg_arr_get_float as *const u8),
        ("spg_arr_get_str", spg_arr_get_str as *const u8),
        ("spg_arr_set", spg_arr_set as *const u8),
        ("spg_struct_set", spg_struct_set as *const u8),
        ("spg_struct_get", spg_struct_get as *const u8),
        ("spg_struct_retain", spg_struct_retain as *const u8),
        ("spg_struct_release", spg_struct_release as *const u8),
        ("spg_struct_new", spg_struct_new as *const u8),
        ("spg_arr_new", spg_arr_new as *const u8),
        ("spg_arr_add", spg_arr_add as *const u8),
        ("spg_arr_push", spg_arr_push as *const u8),
        ("spg_arr_put", spg_arr_put as *const u8),
        ("spg_tarr_new", spg_tarr_new as *const u8),
        ("spg_tarr_retain", spg_tarr_retain as *const u8),
        ("spg_tarr_release", spg_tarr_release as *const u8),
        ("spg_tarr_len", spg_tarr_len as *const u8),
        ("spg_tarr_get", spg_tarr_get as *const u8),
        ("spg_tarr_set", spg_tarr_set as *const u8),
        ("spg_tarr_push", spg_tarr_push as *const u8),
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
    let i3v = sig(&[I64, I64, I64], None);
    let decls: Vec<(&'static str, FuncId)> = vec![
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
        ("spg_print", imp(&mut module, "spg_print", &i3v)?),
        ("spg_arr_get_int", imp(&mut module, "spg_arr_get_int", &i2i)?),
        ("spg_arr_get_bool", imp(&mut module, "spg_arr_get_bool", &i2i)?),
        ("spg_arr_get_float", imp(&mut module, "spg_arr_get_float", &i2f)?),
        ("spg_arr_get_str", imp(&mut module, "spg_arr_get_str", &i2i)?),
        ("spg_arr_set", imp(&mut module, "spg_arr_set", &i4i)?),
        ("spg_struct_set", imp(&mut module, "spg_struct_set", &i4i)?),
        ("spg_struct_get", imp(&mut module, "spg_struct_get", &i3i)?),
        ("spg_struct_retain", imp(&mut module, "spg_struct_retain", &i1v)?),
        ("spg_struct_release", imp(&mut module, "spg_struct_release", &i1v)?),
        ("spg_struct_new", imp(&mut module, "spg_struct_new", &i3i)?),
        ("spg_arr_new", imp(&mut module, "spg_arr_new", &sig(&[], Some(I64)))?),
        ("spg_arr_add", imp(&mut module, "spg_arr_add", &i3i)?),
        ("spg_arr_push", imp(&mut module, "spg_arr_push", &i3i)?),
        ("spg_arr_put", imp(&mut module, "spg_arr_put", &i4i)?),
        ("spg_tarr_new", imp(&mut module, "spg_tarr_new", &sig(&[], Some(I64)))?),
        ("spg_tarr_retain", imp(&mut module, "spg_tarr_retain", &i1v)?),
        ("spg_tarr_release", imp(&mut module, "spg_tarr_release", &i1v)?),
        ("spg_tarr_len", imp(&mut module, "spg_tarr_len", &i1i)?),
        ("spg_tarr_get", imp(&mut module, "spg_tarr_get", &i2i)?),
        ("spg_tarr_set", imp(&mut module, "spg_tarr_set", &i3i)?),
        ("spg_tarr_push", imp(&mut module, "spg_tarr_push", &i2i)?),
    ];

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
        let h = helpers_in(&mut module, &decls, &mut fctx.func);
        {
            let mut b = FunctionBuilder::new(&mut fctx.func, &mut fbc);
            let own = sigs[i].clone().unwrap();
            lower(&mut b, chunk, &own, slots[i].as_ref().unwrap(), &sigs, &program.struct_defs, &refs, h, None, typed[i]);
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
    let table = NativeTable {
        module: RefCell::new(module),
        plain: program.clone(),
        ids,
        sigs: sigs.clone(),
        structs: program.struct_defs.clone(),
        decls,
        call_conv,
        regions: RefCell::new(HashMap::new()),
        counts: RefCell::new(HashMap::new()),
        next_region: Cell::new(0),
        _keep: keep,
        entries,
        arg_owned,
        disabled,
        compiled,
        reasons,
        refused: RefCell::new(HashMap::new()),
        blocks: RefCell::new(HashMap::new()),
    };
    table.compile_blocks()?;
    Ok(table)
}

fn calls(chunk: &Chunk, callee: usize) -> bool {
    chunk.code.iter().any(|op| matches!(op, OpCode::Call { func_index, .. } if *func_index == callee))
}

/// a loop compiled natively, specialized for the kinds its locals had when it was first run
struct Region {
    ptr: usize,
    /// where the loop starts (plain code): its condition
    head: usize,
    nlocals: usize,
    /// (slot, kind) read from the locals on entry
    loads: Vec<(usize, K)>,
    /// (slot, kind) written back to the locals on success
    stores: Vec<(usize, K)>,
}

/// after this many arrivals at a loop top, the loop gets compiled
const HOT_LOOP_ENTRIES: u32 = 50;

fn scalar_kind(v: &Value) -> Option<K> {
    match v {
        Value::Int(_) => Some(K::Int),
        Value::Float(_) => Some(K::Float),
        Value::Bool(_) => Some(K::Bool),
        _ => None,
    }
}

/// the loop `head..exit` as a chunk of its own. jumps to `exit` land one past its end.
/// Err holds the pc of the first jump that leaves the loop by another route
fn region_chunk(plain: &Chunk, head: usize, exit: usize) -> Result<Chunk, usize> {
    let mut rc = plain.clone();
    rc.code = plain.code[head..exit].to_vec();
    rc.locs = Vec::new();
    rc.origin = Vec::new();
    let len = exit - head;
    for (i, op) in rc.code.iter_mut().enumerate() {
        if let Some(t) = op.jump_target_mut() {
            let old = *t;
            *t = if old == exit {
                len
            } else if old >= head && old < exit {
                old - head
            } else {
                return Err(head + i);
            };
        }
    }
    Ok(rc)
}

/// the refusal for a string or array local that the loop `head..exit` assigns
fn assigned_refusal(plain: &Chunk, head: usize, exit: usize, s: usize) -> Refusal {
    let pc = plain.code[head..exit]
        .iter()
        .position(|op| matches!(op, OpCode::SetLocal(x) if *x == s))
        .map_or(exit - 1, |p| head + p);
    at(pc, &plain.code[pc], Reason::HeapAssigned)
}

impl NativeTable {
    /// compiles the loop that ends at `exit` (plain code) in chunk `chunk`, for the kinds of `locals` now.
    /// Err says why the loop can't be native
    fn compile_region(&self, chunk: usize, exit: usize, locals: &[Value]) -> Result<Region, Refusal> {
        let plain = self.plain_chunk(chunk).ok_or_else(|| refusal(Reason::Internal))?;
        let nlocals = plain.num_locals;
        if locals.len() != nlocals || exit == 0 || exit > plain.code.len() {
            return Err(refusal(Reason::Internal));
        }
        // a loop ends with its backward jump, the last instruction before its exit
        let head = match plain.code[exit - 1] {
            OpCode::Jump(h) if h < exit - 1 => h,
            _ => return Err(at(exit - 1, &plain.code[exit - 1], Reason::NotALoop)),
        };
        // a `compile VM` block always runs in the VM: a loop that touches one is never native
        if let Some(&(start, _)) = plain.vm_only.iter().find(|&&(s, e)| s < exit && head < e) {
            return Err(Refusal { pc: Some(start), op: String::new(), reason: Reason::VmBlock });
        }
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        let mut assigned: BTreeSet<usize> = BTreeSet::new();
        for op in &plain.code[head..exit] {
            match op {
                OpCode::GetLocal(s) => {
                    touched.insert(*s);
                }
                OpCode::SetLocal(s) => {
                    touched.insert(*s);
                    assigned.insert(*s);
                }
                _ => {}
            }
        }
        let mut init: Vec<Option<K>> = vec![None; nlocals];
        let mut loads = Vec::new();
        for &s in &touched {
            let k = match &locals[s] {
                // an array is shared with the VM: the region gets its own reference, and its
                // changes are in place already. the array can't be replaced inside the loop.
                Value::Array(a) => {
                    if assigned.contains(&s) {
                        return Err(assigned_refusal(plain, head, exit, s));
                    }
                    K::Array(elem_of(&a.borrow()).ok_or_else(|| refusal(Reason::ArrayElem))?)
                }
                v => match scalar_kind(v) {
                    Some(k) => k,
                    None => continue,
                },
            };
            init[s] = Some(k);
            loads.push((s, k));
        }
        let rc = region_chunk(plain, head, exit).map_err(|pc| at(pc, &plain.code[pc], Reason::JumpOutOfLoop))?;
        let slots = check_init(&rc, &init, K::Int, &self.sigs, &self.structs, false).map_err(|r| Refusal {
            pc: r.pc.map(|p| head + p),
            ..r
        })?;
        let mut stores = Vec::new();
        for &s in &touched {
            let k = slots[s];
            if is_heap(k) {
                // read-only heap locals (arrays) are shared with the VM already: nothing to write back
                if assigned.contains(&s) {
                    return Err(assigned_refusal(plain, head, exit, s));
                }
                continue;
            }
            if !matches!(k, K::Int | K::Float | K::Bool) {
                return Err(refusal(Reason::LocalKind));
            }
            stores.push((s, k));
        }
        let mut loads_mask = vec![false; nlocals];
        for &(s, _) in &loads {
            loads_mask[s] = true;
        }
        let mut stores_mask = vec![false; nlocals];
        for &(s, _) in &stores {
            stores_mask[s] = true;
        }

        let ptr = self.codegen(&rc, &slots, &loads_mask, &stores_mask)?;
        Ok(Region { ptr, head, nlocals, loads, stores })
    }

    /// generates the native code of a region from its chunk and the kinds of its slots. returns its address
    fn codegen(&self, rc: &Chunk, slots: &[K], loads_mask: &[bool], stores_mask: &[bool]) -> Result<usize, Refusal> {
        let mut sig = Signature::new(self.call_conv);
        for _ in 0..3 {
            sig.params.push(AbiParam::new(I64));
        }
        sig.returns.push(AbiParam::new(I64));
        let mut module = self.module.borrow_mut();
        let name = format!("spg_loop_{}", self.next_region.get());
        self.next_region.set(self.next_region.get() + 1);
        let id = module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|_| refusal(Reason::Internal))?;
        let mut fctx = Context::new();
        let mut fbc = FunctionBuilderContext::new();
        fctx.func.signature = sig;
        fctx.func.name = UserFuncName::user(0, id.as_u32());
        let mut refs: HashMap<usize, FuncRef> = HashMap::new();
        for (&callee, &cid) in &self.ids {
            if calls(&rc, callee) {
                refs.insert(callee, module.declare_func_in_func(cid, &mut fctx.func));
            }
        }
        let h = helpers_in(&mut module, &self.decls, &mut fctx.func);
        {
            let mut b = FunctionBuilder::new(&mut fctx.func, &mut fbc);
            lower(
                &mut b,
                &rc,
                &(Vec::<K>::new(), K::Int),
                &slots,
                &self.sigs,
                &self.structs,
                &refs,
                h,
                Some((loads_mask, stores_mask)),
                false,
            );
            b.finalize();
        }
        module.define_function(id, &mut fctx).map_err(|_| refusal(Reason::Internal))?;
        module.clear_context(&mut fctx);
        module.finalize_definitions().map_err(|_| refusal(Reason::Internal))?;
        let ptr = module.get_finalized_function(id) as usize;
        Ok(ptr)
    }

    /// compiles every `compile JIT` block ahead of the run. the static check has passed, so a refusal here is a bug
    fn compile_blocks(&self) -> Result<(), String> {
        for (ci, chunk) in chunks_of(&self.plain) {
            for b in 0..chunk.jit_blocks.len() {
                match self.compile_block(ci, b) {
                    Ok(region) => {
                        self.blocks.borrow_mut().insert((ci, b), Rc::new(region));
                    }
                    Err(r) => {
                        let start = chunk.jit_blocks[b].start;
                        let line = chunk.locs.get(r.pc.unwrap_or(start)).map_or(0, |l| l.line);
                        return Err(format!("line {line}: compile JIT block cannot be native: {}", explain_refusal(&self.plain, &r)));
                    }
                }
            }
        }
        Ok(())
    }

    /// the native code of one `compile JIT` block of chunk `chunk`
    fn compile_block(&self, chunk: usize, block: usize) -> Result<Region, Refusal> {
        let plain = self.plain_chunk(chunk).ok_or_else(|| refusal(Reason::Internal))?;
        let info = plain.jit_blocks.get(block).ok_or_else(|| refusal(Reason::Internal))?;
        let plan = plan_block(plain, info, &self.sigs, &self.structs)?;
        let ptr = self.codegen(&plan.rc, &plan.slots, &plan.loads_mask, &plan.stores_mask)?;
        Ok(Region { ptr, head: info.start, nlocals: plain.num_locals, loads: plan.loads, stores: plan.stores })
    }

    /// the plain chunk with this index (`functions.len()` is main)
    fn plain_chunk(&self, chunk: usize) -> Option<&Chunk> {
        if chunk == self.plain.functions.len() {
            Some(&self.plain.main)
        } else {
            self.plain.functions.get(chunk)
        }
    }

    /// the name of a chunk, for messages
    fn chunk_name(&self, chunk: usize) -> String {
        if chunk == self.plain.functions.len() {
            "main".to_string()
        } else {
            self.plain.function_names.get(chunk).cloned().unwrap_or_else(|| format!("#{chunk}"))
        }
    }

    /// the refusal in words, for the report
    fn explain(&self, r: &Refusal) -> String {
        explain_refusal(&self.plain, r)
    }

    /// the `--jit-report` lines: every function, and every loop the VM has asked about so far
    fn build_report(&self) -> Vec<String> {
        let mut lines = vec!["native code report".to_string(), "  functions:".to_string()];
        for (i, chunk) in self.plain.functions.iter().enumerate() {
            let name = self.chunk_name(i);
            let line = if self.ids.contains_key(&i) {
                format!("    {name}: native")
            } else {
                match self.reasons.get(i).and_then(|r| r.as_ref()) {
                    Some(r) => format!("    {name}{}: not native: {}", line_of(chunk, r.pc), self.explain(r)),
                    None => format!("    {name}: not native"),
                }
            };
            lines.push(line);
        }
        lines.push("  loops:".to_string());
        let regions = self.regions.borrow();
        let counts = self.counts.borrow();
        let refused = self.refused.borrow();
        let mut keys: BTreeSet<(usize, usize)> = regions.keys().copied().collect();
        keys.extend(counts.keys().copied());
        for key in keys {
            let (chunk, exit) = key;
            let Some(plain) = self.plain_chunk(chunk) else { continue };
            let Some(head) = exit
                .checked_sub(1)
                .and_then(|e| plain.code.get(e))
                .and_then(|op| match op {
                    OpCode::Jump(h) => Some(*h),
                    _ => None,
                })
            else {
                continue;
            };
            let state = match (regions.get(&key), refused.get(&key)) {
                (Some(Some(_)), _) => "native".to_string(),
                (_, Some(r)) => format!("not native{}: {}", line_of(plain, r.pc), self.explain(r)),
                (Some(None), None) => "not native".to_string(),
                (None, None) => {
                    let n = counts.get(&key).copied().unwrap_or(0);
                    format!("waiting, reached {n} of {HOT_LOOP_ENTRIES} times")
                }
            };
            lines.push(format!("    loop in {}{}: {state}", self.chunk_name(chunk), line_of(plain, Some(head))));
        }
        lines.push("  compile JIT blocks:".to_string());
        for (ci, chunk) in chunks_of(&self.plain) {
            for (b, info) in chunk.jit_blocks.iter().enumerate() {
                let state = if self.blocks.borrow().contains_key(&(ci, b)) { "native" } else { "not native" };
                lines.push(format!("    block in {}{}: {state}", self.chunk_name(ci), line_of(chunk, Some(info.start))));
            }
        }
        lines
    }
}

/// " (line N)" for the instruction at `pc`, when the chunk knows where it came from
fn line_of(chunk: &Chunk, pc: Option<usize>) -> String {
    match pc.and_then(|p| chunk.locs.get(p)) {
        Some(loc) => format!(" (line {})", loc.line),
        None => String::new(),
    }
}

/// the refusal in words, for messages and the report
pub fn explain_refusal(program: &Program, r: &Refusal) -> String {
    match &r.reason {
        Reason::Signature => "a param or the return type is not int, float, bool, string, or an array of those".to_string(),
        Reason::Arity => format!("takes more than {MAX_ARITY} params"),
        Reason::NoEnd => "does not end with a return or a jump".to_string(),
        Reason::BadJump => "jumps outside its code".to_string(),
        Reason::StackAcrossJump => "leaves a value on the stack across a jump".to_string(),
        Reason::Opcode => format!("`{}` is not a native instruction", r.op),
        Reason::Builtin(name) => format!("builtin `{name}` is not native"),
        Reason::Constant => "uses a constant native code does not hold".to_string(),
        Reason::LocalKind => "a local changes kind, or is read before it has one".to_string(),
        Reason::Operands => format!("the operands of `{}` have the wrong kind", r.op),
        Reason::Stack => "needs more values on the stack than it has".to_string(),
        Reason::Callee(f) => format!("calls `{}`, which is not native", chunk_name_of(program, *f)),
        Reason::ReturnKind => "returns a value of the wrong kind".to_string(),
        Reason::ArrayElem => "uses an array whose elements native code can't hold (empty arrays, arrays of arrays or structs)".to_string(),
        Reason::HeapAssigned => "assigns a string or array local inside the loop".to_string(),
        Reason::StructField => "builds a struct that has a field native code can't hold (an array or a struct)".to_string(),
        Reason::JumpOutOfLoop => "jumps out of the loop by another route than its exit".to_string(),
        Reason::NotALoop => "does not end with its backward jump".to_string(),
        Reason::VmBlock => "touches a `compile VM` block, which always runs in the VM".to_string(),
        Reason::JitBlock => "contains a `compile JIT` block, which runs as native code of its own".to_string(),
        Reason::JumpOutOfBlock => "jumps out of the `compile JIT` block (a break, a continue, or a jump past its end)".to_string(),
        Reason::ReturnInBlock => "returns from inside the `compile JIT` block: a block can't return".to_string(),
        Reason::Internal => "native code generation failed".to_string(),
    }
}

/// the name of a chunk of the plain program, for messages
fn chunk_name_of(program: &Program, chunk: usize) -> String {
    if chunk == program.functions.len() {
        "main".to_string()
    } else {
        program.function_names.get(chunk).cloned().unwrap_or_else(|| format!("#{chunk}"))
    }
}

/// every chunk of the plain program: the functions in order, then main (at index `functions.len()`)
fn chunks_of(program: &Program) -> Vec<(usize, &Chunk)> {
    program.functions.iter().enumerate().chain(std::iter::once((program.functions.len(), &program.main))).collect()
}

/// the native kind of a static slot type
fn slot_kind(t: &spg_vm::chunk::SlotTy) -> K {
    use spg_vm::chunk::SlotTy;
    match t {
        SlotTy::Int => K::Int,
        SlotTy::Float => K::Float,
        SlotTy::Bool => K::Bool,
        SlotTy::String => K::String,
        SlotTy::Array(e) => K::Array(match e.as_ref() {
            SlotTy::Float => ElemKind::Float,
            SlotTy::Bool => ElemKind::Bool,
            SlotTy::String => ElemKind::String,
            _ => ElemKind::Int,
        }),
    }
}

/// a refusal from a region's own pcs, moved to the plain chunk: `by` is where the region starts
fn shift(r: Refusal, by: usize) -> Refusal {
    Refusal { pc: r.pc.map(|p| p + by), ..r }
}

/// the instructions of one `compile JIT` block as a chunk of their own. jumps to the block's end
/// become the chunk's end, and the chunk ends by jumping there, so native code falls out where the block does
fn block_chunk(plain: &Chunk, start: usize, end: usize) -> Result<Chunk, Refusal> {
    let mut rc = plain.clone();
    rc.code = plain.code[start..end].to_vec();
    rc.locs = Vec::new();
    rc.origin = Vec::new();
    rc.jit_blocks = Vec::new();
    rc.vm_only = Vec::new();
    // the block's ops are 0..len, then the closing jump at len, and the exit (where lower stores the
    // locals back) is the pc after that jump: len + 1, which is where a jump to the block's end goes
    let len = end - start;
    for i in 0..rc.code.len() {
        let op = rc.code[i];
        if matches!(op, OpCode::Return) {
            return Err(at(i, &op, Reason::ReturnInBlock));
        }
        if let Some(t) = rc.code[i].jump_target_mut() {
            let old = *t;
            *t = if old == end {
                len + 1
            } else if old >= start && old < end {
                old - start
            } else {
                return Err(at(i, &op, Reason::JumpOutOfBlock));
            };
        }
    }
    rc.code.push(OpCode::Jump(len + 1));
    Ok(rc)
}

/// what one `compile JIT` block needs to run as native code: its region chunk, the kinds of its slots,
/// and what it loads on entry and stores on exit. the same plan serves the static check and the build
struct BlockPlan {
    rc: Chunk,
    slots: Vec<K>,
    loads: Vec<(usize, K)>,
    stores: Vec<(usize, K)>,
    loads_mask: Vec<bool>,
    stores_mask: Vec<bool>,
}

fn plan_block(plain: &Chunk, info: &spg_vm::chunk::JitBlockInfo, sigs: &[Option<Sig>], structs: &[Rc<spg_vm::value::StructDef>]) -> Result<BlockPlan, Refusal> {
    // the block's ops start after its marker: the region is what the marker guards, and pcs are counted from there
    let start = info.start + 1;
    let rc = block_chunk(plain, start, info.end).map_err(|r| shift(r, start))?;
    // the kinds on entry come from the compiler's snapshot of the scope: the program is typed, so they don't depend on the run
    let mut init: Vec<Option<K>> = vec![None; plain.num_locals];
    for (s, t) in info.entry.iter().enumerate() {
        if let (Some(t), Some(slot)) = (t, init.get_mut(s)) {
            *slot = Some(slot_kind(t));
        }
    }
    let slots = check_init(&rc, &init, K::Int, sigs, structs, false).map_err(|r| shift(r, start))?;
    let mut read: BTreeSet<usize> = BTreeSet::new();
    let mut written: BTreeSet<usize> = BTreeSet::new();
    for op in &rc.code {
        match op {
            OpCode::GetLocal(s) => {
                read.insert(*s);
            }
            OpCode::SetLocal(s) => {
                written.insert(*s);
            }
            _ => {}
        }
    }
    // a refusal about slot `s` points at its first write, in plain pcs
    let refuse_write = |s: usize, reason: Reason| {
        let pc = rc.code.iter().position(|op| matches!(op, OpCode::SetLocal(x) if *x == s)).unwrap_or(0);
        shift(at(pc, &rc.code[pc], reason), start)
    };
    let mut loads = Vec::new();
    for &s in &read {
        if let Some(k) = init[s] {
            loads.push((s, k));
        }
    }
    let mut stores = Vec::new();
    for &s in &written {
        let k = slots[s];
        let outer = info.entry.get(s).is_some_and(|t| t.is_some());
        if is_heap(k) {
            // an outer string is handed back to its local with the block's new value. an outer array can't be,
            // yet: the VM would keep the old one
            if outer && k == K::String {
                stores.push((s, k));
            } else if outer {
                return Err(refuse_write(s, Reason::HeapAssigned));
            }
            continue;
        }
        if !matches!(k, K::Int | K::Float | K::Bool) {
            return Err(refuse_write(s, Reason::LocalKind));
        }
        stores.push((s, k));
    }
    let mut loads_mask = vec![false; plain.num_locals];
    for (s, _) in &loads {
        loads_mask[*s] = true;
    }
    let mut stores_mask = vec![false; plain.num_locals];
    for (s, _) in &stores {
        stores_mask[*s] = true;
    }
    Ok(BlockPlan { rc, slots, loads, stores, loads_mask, stores_mask })
}

/// the static check of `compile JIT`: every block must be native code. the first block that can't be is an
/// error, with its line and why. `spg check` and `spg run` both run it, before anything executes
pub fn check_blocks(program: &Program) -> Result<(), String> {
    let (elig, _, _, _) = eligible(program);
    let sigs: Vec<Option<Sig>> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, c)| if elig[i] { c.native_sig.clone() } else { None })
        .collect();
    for (_, chunk) in chunks_of(program) {
        for info in &chunk.jit_blocks {
            if let Err(r) = plan_block(chunk, info, &sigs, &program.struct_defs) {
                let line = chunk.locs.get(r.pc.unwrap_or(info.start)).map_or(0, |l| l.line);
                return Err(format!("line {line}: compile JIT block cannot be native: {}", explain_refusal(program, &r)));
            }
        }
    }
    Ok(())
}
