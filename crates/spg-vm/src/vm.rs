//! The VM. A dumb loop over Vec<OpCode>. Synchronous on purpose.
//! Threads? Later. Maybe. Don't ask me at 3am.

use std::rc::Rc;

use crate::builtins::call_builtin_on_stack;
use crate::field_ops;
use crate::index_ops;
use crate::chunk::{Chunk, NativeKind, OpCode, Program};
use crate::arith;
use crate::error::RuntimeError;
use crate::value::{Closure, Value};

/// One function call. Locals get their own Vec per frame.
/// The operand stack is ONE shared Vec for the whole call chain, borrowed
/// by ref, because allocating a stack per call made the benchmarks cry.
struct CallFrame<'p, 'v> {
    chunk: &'p Chunk,
    ip: usize,
    locals: Vec<Value>,
    /// shared by the whole chain. A frame only pops what it pushed (or what
    /// its callees pushed). Don't break that. I beg you.
    value_stack: &'v mut Vec<Value>,
    /// active `try` handlers in this frame, innermost last. each entry is
    /// (catch block's instruction index, operand-stack height when the try
    /// began). the height lets an exception that lands mid-expression (a callee
    /// threw) drop the half-built temporaries first. dont leave garbage on the stack.
    /// no handler here? the frame returns an
    /// `Err` and the caller's `run_chunk` checks ITS handlers. that's how a
    /// throw reaches a try in some other function. magic, basically.
    handlers: Vec<(usize, usize)>,
}

impl<'p, 'v> CallFrame<'p, 'v> {
    fn new(chunk: &'p Chunk, args: Vec<Value>, value_stack: &'v mut Vec<Value>) -> Self {
        let mut locals = args;
        locals.resize(chunk.num_locals, Value::Void);
        CallFrame { chunk, ip: 0, locals, value_stack, handlers: Vec::new() }
    }

    fn push(&mut self, v: Value) {
        self.value_stack.push(v);
    }

    fn pop(&mut self) -> Result<Value, RuntimeError> {
        self.value_stack.pop().ok_or(RuntimeError::StackUnderflow)
    }
}

pub struct Vm<'p> {
    program: &'p Program,
    /// recycled Vecs for call frame locals: a call grabs one (already
    /// allocated) instead of asking the allocator, and gives it back on return.
    pool: Vec<Vec<Value>>,
    /// every root-scope variable of the whole program. indexed by the
    /// slot the compiler gave it. one array for the whole run, shared
    /// by every frame. see OpCode::GetGlobal. don't ask why it's global.
    globals: Vec<Value>,
    /// the call chain of the error being thrown up, innermost frame first.
    /// filled while the error unwinds. cleared when a try catches it. good riddance.
    trace: Vec<TraceFrame>,
    /// frames that didn't fit in `trace` (deep recursion keeps only
    /// the innermost ones; the count says how many got cut)
    trace_omitted: usize,
    /// optional native backend, asked before running an `int` function.
    native: Option<Box<dyn NativeCalls>>,
    /// how many frames are running right now (every spg call is a native recursion, yay)
    depth: usize,
}

/// Max spg call depth. The CLI gives the VM a 1 GiB stack, so this fits fine.
/// Without the limit, infinite recursion just kills the process with no trace,
/// and nobody wants that at 4am.
pub const MAX_CALL_DEPTH: usize = 100_000;

/// native version of some `int`-only functions (see spg-jit). fast, and fragile.
/// `call` returns `None` when native can't answer for sure (overflow,
/// div by zero, too deep, a callee that gave up...): the VM runs the
/// function itself. always correct, because these functions are pure. phew.
pub trait NativeCalls {
    /// does this function have native code at all? checked before any arg work.
    fn has(&self, func_index: usize) -> bool;
    fn call(&self, func_index: usize, args: &[i64]) -> Option<i64>;
}

/// how many frames a trace keeps before it starts just counting
const TRACE_LIMIT: usize = 50;

/// one line of a trace: the function, and where in the source it died
#[derive(Debug, Clone, PartialEq)]
pub struct TraceFrame {
    /// `<main>` for top-level code, otherwise the function name
    pub function: String,
    pub file: Option<String>,
    pub line: u32,
}

/// what one step produced. right now it's only "the
/// frame returned with this value". that's the whole thing.
enum FrameOutcome {
    Returned(Value),
}

impl<'p> Vm<'p> {
    pub fn new(program: &'p Program) -> Self {
        crate::builtins::mark_program_start();
        crate::value::install_enum_defs(&program.enum_defs);
        Vm {
            program,
            pool: Vec::with_capacity(64),
            globals: vec![Value::Void; program.num_globals],
            trace: Vec::new(),
            trace_omitted: 0,
            depth: 0,
            native: None,
        }
    }

    /// installs a native backend. int calls try it first.
    pub fn set_native(&mut self, native: Box<dyn NativeCalls>) {
        self.native = Some(native);
    }

    /// tries native for a call whose args are on the stack.
    /// on success, the args get replaced by the result and we return `true`.
    fn try_native(&self, frame: &mut CallFrame<'p, '_>, func_index: usize, argc: usize) -> bool {
        let Some(native) = self.native.as_ref() else { return false };
        if !native.has(func_index) || argc > 4 {
            return false;
        }
        let Some((params, ret)) = self.program.functions.get(func_index).and_then(|c| c.native_sig.as_ref()) else {
            return false;
        };
        let at = frame.value_stack.len() - argc;
        // check every kind before converting anything. a string arg hands
        // native a reference, which must not dangle if the kinds don't match. don't be sloppy.
        let fits = frame.value_stack[at..].iter().zip(params).all(|(v, k)| {
            matches!(
                (v, k),
                (Value::Int(_), NativeKind::Int)
                    | (Value::Bool(_), NativeKind::Bool)
                    | (Value::Float(_), NativeKind::Float)
                    | (Value::String(_), NativeKind::String)
                    | (Value::Array(_), NativeKind::Array(_))
            ) || matches!((v, k), (Value::Struct(s), NativeKind::Struct(d))
                if Rc::ptr_eq(&s.borrow().0, &self.program.struct_defs[*d]))
        });
        if !fits {
            return false;
        }
        // native takes every value as its 64-bit pattern: floats as their
        // bits, strings as an owned pointer (one extra ref, native releases it)
        let mut buf = [0i64; 4];
        for (i, v) in frame.value_stack[at..].iter().enumerate() {
            buf[i] = match v {
                Value::Int(n) => *n,
                Value::Bool(b) => *b as i64,
                Value::Float(f) => f.to_bits() as i64,
                Value::String(s) => Rc::into_raw(s.clone()) as i64,
                Value::Array(a) => Rc::into_raw(a.clone()) as i64,
                Value::Struct(s) => Rc::into_raw(s.clone()) as i64,
                _ => unreachable!("kinds were checked above"),
            };
        }
        match native.call(func_index, &buf[..argc]) {
            Some(r) => {
                frame.value_stack.truncate(at);
                frame.push(match ret {
                    NativeKind::Int => Value::Int(r),
                    NativeKind::Bool => Value::Bool(r != 0),
                    NativeKind::Float => Value::Float(f64::from_bits(r as u64)),
                    // arrays never come back from native (native_sig rejects them). thank god.
                    NativeKind::Array(_) | NativeKind::Struct(_) => return false,
                    NativeKind::String if r != 0 => {
                        // SAFETY: native returns an owned ref made by Rc::into_raw. trust me bro.
                        Value::String(unsafe { Rc::from_raw(r as *const String) })
                    }
                    NativeKind::String => return false,
                });
                true
            }
            None => false,
        }
    }

    /// runs `main` to completion and returns
    /// whatever it left on top of the stack (always `Value::Void` in practice,
    /// since `main` can't produce a value in spg. dead end, but kept for later.
    /// (kept as a real return in case that changes)
    ///
    /// the operand stack is one `Vec<Value>` shared across every
    /// call in the run, not one per frame. that's what lets a
    /// call skip allocating a stack. I tried this and a
    /// few other ideas (an explicit call stack instead of native
    /// recursion, shrinking `Value`) in SpgLab, a throwaway copy of the
    /// VM I use to test perf ideas. most of them were slower. learned that the hard way.
    pub fn run(&mut self) -> Result<Value, RuntimeError> {
        self.trace.clear();
        self.trace_omitted = 0;
        let mut value_stack: Vec<Value> = Vec::with_capacity(256);
        let result = self.run_chunk(&self.program.main, Vec::new(), &mut value_stack);
        // buffered `print` output has to land before the caller reports anything. obviously.
        crate::builtins::flush_stdout();
        result
    }

    /// runs one chunk (a function body or `main`) to completion, given
    /// its already-evaluated args. nested spg calls recurse into
    /// this again, so pathologically deep spg recursion will
    /// eventually hit a Rust stack overflow instead of a clean spg error.
    /// yes, that's deliberate. no, I'm not fixing it today.
    /// `value_stack` gets passed by ref instead of living
    /// on `Vm` itself, which avoids a borrow fight on the recursive
    /// call. the borrow checker and I have an understanding.
    /// `ps.run(cmd [, timeout])` and `ps.runFile(path [, timeout])`: runs PowerShell and
    /// builds the `PsResult` the program declared (success, code, output, error). in that order, don't swap.
    fn ps_run(&self, frame: &mut CallFrame<'p, '_>, id: spg_typecheck::BuiltinId, argc: usize) -> Result<Value, RuntimeError> {
        let function = if id == spg_typecheck::BuiltinId::PsRunFile { "ps.runFile" } else { "ps.run" };
        let mut args: Vec<Value> = (0..argc).map(|_| frame.pop()).collect::<Result<_, _>>()?;
        args.reverse();
        let text = match args.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => return Err(RuntimeError::TypeMismatch { expected: "string", found: "other" }),
        };
        let timeout = match args.get(1) {
            Some(Value::Int(n)) => *n,
            _ => 30,
        };
        // the folder of the script that made the call (the frame's own file). relative paths are a pain.
        let here = frame.chunk.locs.get(frame.ip.saturating_sub(1)).and_then(|loc| loc.file.as_deref());
        let (mode, arg) = if id == spg_typecheck::BuiltinId::PsRunFile {
            let path = std::path::Path::new(&text);
            let escapes = path.components().any(|c| {
                matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_))
            });
            if crate::powershell::safe_party() && escapes {
                return Err(RuntimeError::Process {
                    function,
                    reason: "blocked by #> safe-party: runFile only takes a .ps1 inside the script's folder (no absolute paths, no ..)".to_string(),
                });
            }
            let resolved = if path.is_absolute() {
                path.to_path_buf()
            } else {
                here.map(|file| std::path::Path::new(file).parent().unwrap_or(std::path::Path::new(".")).join(path))
                    .unwrap_or_else(|| path.to_path_buf())
            };
            ("file", resolved.to_string_lossy().into_owned())
        } else {
            ("cmd", text)
        };
        let done = crate::powershell::exec(function, mode, &arg, timeout)?;
        let def = self
            .program
            .struct_defs
            .iter()
            .find(|d| d.name == "PsResult")
            .cloned()
            .ok_or(RuntimeError::Process { function, reason: "PsResult is not declared (internal error)".to_string() })?;
        let values = vec![
            Value::Bool(done.code == 0),
            Value::Int(done.code),
            Value::String(Rc::new(done.output)),
            Value::String(Rc::new(done.error)),
        ];
        Ok(Value::Struct(Rc::new(std::cell::RefCell::new((def, values)))))
    }

    fn run_chunk(&mut self, chunk: &'p Chunk, args: Vec<Value>, value_stack: &mut Vec<Value>) -> Result<Value, RuntimeError> {
        if self.depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError::StackOverflow { limit: MAX_CALL_DEPTH });
        }
        self.depth += 1;
        let mut frame = CallFrame::new(chunk, args, value_stack);

        loop {
            match self.step(&mut frame) {
                Ok(Some(FrameOutcome::Returned(v))) => {
                    let mut locals = std::mem::take(&mut frame.locals);
                    locals.clear();
                    self.pool.push(locals);
                    self.depth -= 1;
                    return Ok(v);
                }
                Ok(None) => {} // keep looping
                Err(e) => {
                    if let Err(e) = Self::catch_or_propagate(&mut frame, e) {
                        self.record_frame(&frame);
                        self.depth -= 1;
                        return Err(e);
                    }
                    // caught here. whatever the deeper frames recorded belonged to
                    // an error that's handled now, so it's not a trace anymore. bye.
                    self.trace.clear();
                    self.trace_omitted = 0;
                }
            }
        }
    }

    /// adds this frame to the trace of the error leaving it
    #[cold]
    #[inline(never)]
    fn record_frame(&mut self, frame: &CallFrame<'p, '_>) {
        if self.trace.len() >= TRACE_LIMIT {
            self.trace_omitted += 1;
            return;
        }
        let loc = frame.chunk.locs.get(frame.ip.saturating_sub(1)).cloned().unwrap_or_default();
        let is_main = std::ptr::eq(frame.chunk, &self.program.main);
        self.trace.push(TraceFrame {
            function: if is_main {
                "<main>".to_string()
            } else {
                frame.chunk.name.rsplit("::").next().unwrap_or_default().to_string()
            },
            file: loc.file.as_deref().map(str::to_string),
            line: loc.line,
        });
    }

    /// the trace of the last error `run` returned, innermost first, plus how
    /// many frames got cut. empty after a successful run, obviously.
    pub fn take_trace(&mut self) -> (Vec<TraceFrame>, usize) {
        let omitted = std::mem::take(&mut self.trace_omitted);
        (std::mem::take(&mut self.trace), omitted)
    }

    /// an error showed up in `frame`. if it's catchable (a throw, or a runtime
    /// thing like a div by zero or a missing map key) and this frame
    /// has an active try, jump to its catch with the message on the stack.
    /// otherwise pass it up. the caller gets the same chance.
    /// VM bugs are never catchable. that's on purpose. kept out of the hot loop.
    #[cold]
    #[inline(never)]
    fn catch_or_propagate(frame: &mut CallFrame<'p, '_>, e: RuntimeError) -> Result<(), RuntimeError> {
        let Some(message) = e.catchable_message() else { return Err(e) };
        match frame.handlers.pop() {
            Some((catch_target, depth)) => {
                frame.value_stack.truncate(depth);
                frame.push(Value::String(Rc::new(message)));
                frame.ip = catch_target;
                Ok(())
            }
            None => Err(e),
        }
    }

    /// executes one instruction. returns `Some(Returned(v))` once
    /// `Return` fires, `None` otherwise. `ip` moves by one each call
    /// except jumps, which set it themselves.
    #[inline(always)]
    fn step(&mut self, frame: &mut CallFrame<'p, '_>) -> Result<Option<FrameOutcome>, RuntimeError> {
        let op = frame.chunk.code[frame.ip];
        frame.ip += 1;

        match op {
            OpCode::Const(idx) => frame.push(frame.chunk.constants[idx].clone()),
            OpCode::PushVoid => frame.push(Value::Void),

            OpCode::GetGlobal(slot) => frame.push(self.globals[slot].clone()),
            OpCode::SetGlobal(slot) => {
                // same no-pop deal as SetLocal
                let v = frame.value_stack.last().ok_or(RuntimeError::StackUnderflow)?.clone();
                self.globals[slot] = v;
            }

            OpCode::GetLocal(slot) => frame.push(frame.locals[slot].clone()),
            OpCode::SetLocal(slot) => {
                // doesn't pop. the compiler always emits a Pop right
                // after, once the assignment's value has done its job. 
                // Leaving it on the stack keeps room for a
                // future "assignment as expression" without changing
                // this opcode's format. future me's problem.
                let v = frame.value_stack.last().ok_or(RuntimeError::StackUnderflow)?.clone();
                frame.locals[slot] = v;
            }

            // arithmetic: operands pop right-then-left, since the
            // compiler pushes left then right. don't touch the order.
            OpCode::AddInt => binary_int(frame, |a, b| arith::add(a, b))?,
            OpCode::AddLocals(a, b) => fused_int_arith(frame, a, b, |x, y| arith::add(x, y))?,
            OpCode::SubLocals(a, b) => fused_int_arith(frame, a, b, |x, y| arith::sub(x, y))?,
            OpCode::MulLocals(a, b) => fused_int_arith(frame, a, b, |x, y| arith::mul(x, y))?,
            OpCode::DivLocals(a, b) => fused_int_arith(frame, a, b, |x, y| arith::div(x, y))?,
            OpCode::ModLocals(a, b) => fused_int_arith(frame, a, b, |x, y| arith::rem(x, y))?,
            OpCode::SubInt => binary_int(frame, |a, b| arith::sub(a, b))?,
            OpCode::MulInt => binary_int(frame, |a, b| arith::mul(a, b))?,
            OpCode::DivInt => binary_int(frame, |a, b| arith::div(a, b))?,
            OpCode::ModInt => binary_int(frame, |a, b| arith::rem(a, b))?,
            // the typechecker only allows int**int with non-mixed types, so a
            // negative exponent is a valid spg program, not a bug (see arith::pow)
            OpCode::PowInt => binary_int(frame, |a, b| arith::pow(a, b))?,
            OpCode::AddFloat => binary_float(frame, |a, b| a + b)?,
            OpCode::SubFloat => binary_float(frame, |a, b| a - b)?,
            OpCode::MulFloat => binary_float(frame, |a, b| a * b)?,
            OpCode::DivFloat => binary_float(frame, |a, b| a / b)?,
            OpCode::ModFloat => binary_float(frame, |a, b| a % b)?,
            OpCode::PowFloat => binary_float(frame, |a, b| a.powf(b))?,
            OpCode::ConcatString => {
                let b = frame.pop()?;
                let a = frame.pop()?;
                let (a, b) = (expect_string(a)?, expect_string(b)?);
                frame.push(Value::String(Rc::new(format!("{}{}", a, b))));
            }
            OpCode::NegInt => {
                let v = expect_int(frame.pop()?)?;
                frame.push(Value::Int(arith::neg(v)?));
            }
            OpCode::NegFloat => {
                let v = expect_float(frame.pop()?)?;
                frame.push(Value::Float(-v));
            }

            // ---- comparison (the fun part) ----
            OpCode::EqInt => binary_int_cmp(frame, |a, b| a == b)?,
            OpCode::EqFloat => binary_float_cmp(frame, |a, b| a == b)?,
            OpCode::EqBool => {
                let b = expect_bool(frame.pop()?)?;
                let a = expect_bool(frame.pop()?)?;
                frame.push(Value::Bool(a == b));
            }
            OpCode::EqString => {
                let b = expect_string(frame.pop()?)?;
                let a = expect_string(frame.pop()?)?;
                frame.push(Value::Bool(a == b));
            }
            OpCode::NotEqInt => binary_int_cmp(frame, |a, b| a != b)?,
            OpCode::NotEqFloat => binary_float_cmp(frame, |a, b| a != b)?,
            OpCode::NotEqBool => {
                let b = expect_bool(frame.pop()?)?;
                let a = expect_bool(frame.pop()?)?;
                frame.push(Value::Bool(a != b));
            }
            OpCode::NotEqString => {
                let b = expect_string(frame.pop()?)?;
                let a = expect_string(frame.pop()?)?;
                frame.push(Value::Bool(a != b));
            }
            OpCode::LtInt => binary_int_cmp(frame, |a, b| a < b)?,
            OpCode::GtInt => binary_int_cmp(frame, |a, b| a > b)?,
            OpCode::LtEqInt => binary_int_cmp(frame, |a, b| a <= b)?,
            OpCode::GtEqInt => binary_int_cmp(frame, |a, b| a >= b)?,
            OpCode::LtFloat => binary_float_cmp(frame, |a, b| a < b)?,
            OpCode::GtFloat => binary_float_cmp(frame, |a, b| a > b)?,
            OpCode::LtEqFloat => binary_float_cmp(frame, |a, b| a <= b)?,
            OpCode::GtEqFloat => binary_float_cmp(frame, |a, b| a >= b)?,

            // fused GetLocal(a); GetLocal(b); <CmpInt>. same idea as
            // AddLocals above, extended to comparisons because they're the
            // backbone of every if/while. basically the whole language.
            OpCode::LtLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x < y)?,
            OpCode::GtLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x > y)?,
            OpCode::LtEqLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x <= y)?,
            OpCode::GtEqLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x >= y)?,
            OpCode::EqIntLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x == y)?,
            OpCode::NotEqIntLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x != y)?,

            // same fusion, mirrored for float/string/bool. copy paste, sorry.
            OpCode::AddFloatLocals(a, b) => fused_float_arith(frame, a, b, |x, y| x + y)?,
            OpCode::SubFloatLocals(a, b) => fused_float_arith(frame, a, b, |x, y| x - y)?,
            OpCode::MulFloatLocals(a, b) => fused_float_arith(frame, a, b, |x, y| x * y)?,
            OpCode::DivFloatLocals(a, b) => fused_float_arith(frame, a, b, |x, y| x / y)?,
            OpCode::ModFloatLocals(a, b) => fused_float_arith(frame, a, b, |x, y| x % y)?,
            OpCode::LtFloatLocals(a, b) => fused_float_cmp(frame, a, b, |x, y| x < y)?,
            OpCode::GtFloatLocals(a, b) => fused_float_cmp(frame, a, b, |x, y| x > y)?,
            OpCode::LtEqFloatLocals(a, b) => fused_float_cmp(frame, a, b, |x, y| x <= y)?,
            OpCode::GtEqFloatLocals(a, b) => fused_float_cmp(frame, a, b, |x, y| x >= y)?,
            OpCode::EqFloatLocals(a, b) => fused_float_cmp(frame, a, b, |x, y| x == y)?,
            OpCode::NotEqFloatLocals(a, b) => fused_float_cmp(frame, a, b, |x, y| x != y)?,
            OpCode::ConcatStringLocals(a, b) => fused_string_concat(frame, a, b)?,
            OpCode::EqStringLocals(a, b) => fused_string_cmp(frame, a, b, |x, y| x == y)?,
            OpCode::NotEqStringLocals(a, b) => fused_string_cmp(frame, a, b, |x, y| x != y)?,
            OpCode::EqBoolLocals(a, b) => fused_bool_cmp(frame, a, b, |x, y| x == y)?,
            OpCode::NotEqBoolLocals(a, b) => fused_bool_cmp(frame, a, b, |x, y| x != y)?,

            OpCode::Not => {
                let v = expect_bool(frame.pop()?)?;
                frame.push(Value::Bool(!v));
            }

            OpCode::MakeArray(count) => {
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(frame.pop()?);
                }
                items.reverse(); // popped in reverse push order
                frame.push(Value::Array(Rc::new(std::cell::RefCell::new(items))));
            }
            OpCode::EnumValue => match frame.pop()? {
                Value::Enum(def, variant) => {
                    let value = self.program.enum_defs[def as usize].values[variant as usize].clone();
                    frame.push(value);
                }
                other => return Err(RuntimeError::TypeMismatch { expected: "enum", found: other.type_name() }),
            },
            OpCode::EqValue => {
                let b = frame.pop()?;
                let a = frame.pop()?;
                frame.push(Value::Bool(a.structurally_equal(&b)));
            }
            OpCode::NotEqValue => {
                let b = frame.pop()?;
                let a = frame.pop()?;
                frame.push(Value::Bool(!a.structurally_equal(&b)));
            }
            OpCode::MakeMap(pairs) => {
                let mut flat = Vec::with_capacity(pairs * 2);
                for _ in 0..pairs * 2 {
                    flat.push(frame.pop()?);
                }
                flat.reverse(); // popped in reverse push order
                let mut map = crate::value::SpgMap::default();
                for pair in flat.chunks_exact(2) {
                    if !map.insert(pair[0].clone(), pair[1].clone()) {
                        return Err(RuntimeError::TypeMismatch { expected: "int, string, bool or enum key", found: pair[0].type_name() });
                    }
                }
                frame.push(Value::Map(Rc::new(std::cell::RefCell::new(map))));
            }
            OpCode::MapGet => {
                let key = frame.pop()?;
                let map = expect_map(frame.pop()?)?;
                let found = map.borrow().get(&key).cloned();
                match found {
                    Some(v) => frame.push(v),
                    None => return Err(RuntimeError::KeyNotFound { key: key_text(&key) }),
                }
            }
            OpCode::MapSet => {
                let value = frame.pop()?;
                let key = frame.pop()?;
                let map = expect_map(frame.pop()?)?;
                if !map.borrow_mut().insert(key.clone(), value) {
                    return Err(RuntimeError::TypeMismatch { expected: "int, string, bool or enum key", found: key.type_name() });
                }
            }
            OpCode::IndexGet => {
                let index = expect_int(frame.pop()?)?;
                let array = expect_array(frame.pop()?)?;
                let items = array.borrow();
                if index < 0 || index as usize >= items.len() {
                    return Err(RuntimeError::IndexOutOfBounds { index, len: items.len() });
                }
                frame.push(items[index as usize].clone());
            }
            OpCode::IndexSet => {
                let value = frame.pop()?;
                let index = expect_int(frame.pop()?)?;
                let array = expect_array(frame.pop()?)?;
                let mut items = array.borrow_mut();
                if index < 0 || index as usize >= items.len() {
                    return Err(RuntimeError::IndexOutOfBounds { index, len: items.len() });
                }
                items[index as usize] = value;
            }
            OpCode::ArrayLen => {
                let array = expect_array(frame.pop()?)?;
                frame.push(Value::Int(array.borrow().len() as i64));
            }

            OpCode::MakeStruct { def_index, field_count } => {
                let mut values = Vec::with_capacity(field_count);
                for _ in 0..field_count {
                    values.push(frame.pop()?);
                }
                values.reverse(); // popped in reverse push order, same as MakeArray
                let def = self.program.struct_defs[def_index].clone();
                frame.push(Value::Struct(Rc::new(std::cell::RefCell::new((def, values)))));
            }
            OpCode::FieldGet { field_index } => {
                let s = expect_struct(frame.pop()?)?;
                let value = s.borrow().1[field_index].clone();
                frame.push(value);
            }
            OpCode::FieldSet { field_index } => {
                let value = frame.pop()?;
                let s = expect_struct(frame.pop()?)?;
                s.borrow_mut().1[field_index] = value;
            }

            OpCode::Jump(target) => frame.ip = target,
            OpCode::JumpIfFalse(target) => {
                let cond = expect_bool(frame.pop()?)?;
                if !cond {
                    frame.ip = target;
                }
            }
            OpCode::JumpIfNotNone(target) => {
                if matches!(frame.value_stack.last(), Some(Value::Nil)) {
                    frame.pop()?;
                } else {
                    frame.ip = target;
                }
            }

            OpCode::Call { func_index, argc } => {
                if self.try_native(frame, func_index, argc) {
                    return Ok(None);
                }
                let callee =
                    self.program.functions.get(func_index).ok_or(RuntimeError::InvalidFunctionIndex(func_index))?;
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.reserve(callee.num_locals);
                // args are the top argc values, already in order. yes, it's that simple
                let at = frame.value_stack.len() - argc;
                locals.extend(frame.value_stack.drain(at..));
                let result = self.run_chunk(callee, locals, frame.value_stack)?;
                frame.push(result);
            }
            OpCode::MakeClosure { func_index, ncap } => {
                // captured values are the top ncap stack values, in order
                let at = frame.value_stack.len() - ncap;
                let captures: Vec<Value> = frame.value_stack.drain(at..).collect();
                frame.push(Value::Func(Rc::new(Closure { index: func_index, captures })));
            }
            OpCode::CallValue { argc } => {
                // stack: callee, arg1 .. argN (callee below the args, as usual)
                let at = frame.value_stack.len() - argc;
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.extend(frame.value_stack.drain(at..));
                let callee_value = frame.pop()?;
                let Value::Func(closure) = callee_value else {
                    return Err(RuntimeError::TypeMismatch { expected: "function", found: callee_value.type_name() });
                };
                // captured values go before the args
                let mut all = Vec::with_capacity(closure.captures.len() + locals.len());
                all.extend(closure.captures.iter().cloned());
                all.append(&mut locals);
                self.pool.push(locals);
                let callee = self.program.functions.get(closure.index).ok_or(RuntimeError::InvalidFunctionIndex(closure.index))?;
                let result = self.run_chunk(callee, all, frame.value_stack)?;
                frame.push(result);
            }
            OpCode::CallLocals2 { func_index, a, b } => {
                let callee =
                    self.program.functions.get(func_index).ok_or(RuntimeError::InvalidFunctionIndex(func_index))?;
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.reserve(callee.num_locals);
                locals.push(frame.locals[a].clone());
                locals.push(frame.locals[b].clone());
                let result = self.run_chunk(callee, locals, frame.value_stack)?;
                frame.push(result);
            }
            OpCode::CallLocal1 { func_index, a } => {
                let callee =
                    self.program.functions.get(func_index).ok_or(RuntimeError::InvalidFunctionIndex(func_index))?;
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.reserve(callee.num_locals);
                locals.push(frame.locals[a].clone());
                let result = self.run_chunk(callee, locals, frame.value_stack)?;
                frame.push(result);
            }
            OpCode::CallLocalMinusConst1 { func_index, a, k } => {
                let callee =
                    self.program.functions.get(func_index).ok_or(RuntimeError::InvalidFunctionIndex(func_index))?;
                // same as a plain SubInt, just fused into the call. 
                let n = match &frame.locals[a] {
                    Value::Int(n) => *n,
                    other => return Err(RuntimeError::TypeMismatch { expected: "int", found: other.type_name() }),
                };
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.reserve(callee.num_locals);
                locals.push(Value::Int(arith::sub(n, k)?));
                let result = self.run_chunk(callee, locals, frame.value_stack)?;
                frame.push(result);
            }
            OpCode::Return => {
                let v = frame.pop()?;
                return Ok(Some(FrameOutcome::Returned(v)));
            }

            OpCode::CallBuiltin { id, argc } if matches!(id, spg_typecheck::BuiltinId::PsRun | spg_typecheck::BuiltinId::PsRunFile) => {
                let value = self.ps_run(frame, id, argc)?;
                frame.push(value);
            }
            OpCode::CallBuiltin { id, argc } => call_builtin_on_stack(frame.value_stack, id, argc)?,

            OpCode::Pop => {
                frame.pop()?;
            }
            OpCode::Dup1 => {
                let top = frame.value_stack.last().ok_or(RuntimeError::StackUnderflow)?.clone();
                frame.push(top);
            }
            OpCode::Dup2 => {
                let len = frame.value_stack.len();
                if len < 2 {
                    return Err(RuntimeError::StackUnderflow);
                }
                let a = frame.value_stack[len - 2].clone();
                let b = frame.value_stack[len - 1].clone();
                frame.push(a);
                frame.push(b);
            }

            // ---- superinstructions (we got greedy) ----
            OpCode::StoreLocal(slot) => {
                let v = frame.pop()?;
                frame.locals[slot] = v;
            }
            OpCode::StoreGlobal(slot) => {
                let v = frame.pop()?;
                self.globals[slot] = v;
            }
            OpCode::AddLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Int(arith::add(x, k)?));
            }
            OpCode::MulLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Int(arith::mul(x, k)?));
            }
            OpCode::DivLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Int(arith::div(x, k)?));
            }
            OpCode::ModLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Int(arith::rem(x, k)?));
            }
            OpCode::IncLocal(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.locals[a] = Value::Int(arith::add(x, k)?);
            }
            OpCode::AddConst(k) => {
                let x = expect_int(frame.pop()?)?;
                frame.push(Value::Int(arith::add(x, k)?));
            }
            OpCode::MulConst(k) => {
                let x = expect_int(frame.pop()?)?;
                frame.push(Value::Int(arith::mul(x, k)?));
            }
            OpCode::DivConst(k) => {
                let x = expect_int(frame.pop()?)?;
                frame.push(Value::Int(arith::div(x, k)?));
            }
            OpCode::ModConst(k) => {
                let x = expect_int(frame.pop()?)?;
                frame.push(Value::Int(arith::rem(x, k)?));
            }
            OpCode::LtLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Bool(x < k));
            }
            OpCode::JumpUnlessLt(t) => {
                let b = expect_int(frame.pop()?)?;
                let a = expect_int(frame.pop()?)?;
                if !(a < b) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessLtLocals(a, b, t) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                if !(x < y) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessLtLocalConst(a, k, t) => {
                let x = local_int(&frame.locals, a)?;
                if !(x < k) {
                    frame.ip = t;
                }
            }
            OpCode::GtLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Bool(x > k));
            }
            OpCode::JumpUnlessGt(t) => {
                let b = expect_int(frame.pop()?)?;
                let a = expect_int(frame.pop()?)?;
                if !(a > b) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessGtLocals(a, b, t) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                if !(x > y) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessGtLocalConst(a, k, t) => {
                let x = local_int(&frame.locals, a)?;
                if !(x > k) {
                    frame.ip = t;
                }
            }
            OpCode::LtEqLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Bool(x <= k));
            }
            OpCode::JumpUnlessLtEq(t) => {
                let b = expect_int(frame.pop()?)?;
                let a = expect_int(frame.pop()?)?;
                if !(a <= b) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessLtEqLocals(a, b, t) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                if !(x <= y) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessLtEqLocalConst(a, k, t) => {
                let x = local_int(&frame.locals, a)?;
                if !(x <= k) {
                    frame.ip = t;
                }
            }
            OpCode::GtEqLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Bool(x >= k));
            }
            OpCode::JumpUnlessGtEq(t) => {
                let b = expect_int(frame.pop()?)?;
                let a = expect_int(frame.pop()?)?;
                if !(a >= b) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessGtEqLocals(a, b, t) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                if !(x >= y) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessGtEqLocalConst(a, k, t) => {
                let x = local_int(&frame.locals, a)?;
                if !(x >= k) {
                    frame.ip = t;
                }
            }
            OpCode::EqLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Bool(x == k));
            }
            OpCode::JumpUnlessEq(t) => {
                let b = expect_int(frame.pop()?)?;
                let a = expect_int(frame.pop()?)?;
                if !(a == b) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessEqLocals(a, b, t) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                if !(x == y) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessEqLocalConst(a, k, t) => {
                let x = local_int(&frame.locals, a)?;
                if !(x == k) {
                    frame.ip = t;
                }
            }
            OpCode::NotEqLocalConst(a, k) => {
                let x = local_int(&frame.locals, a)?;
                frame.push(Value::Bool(x != k));
            }
            OpCode::JumpUnlessNotEq(t) => {
                let b = expect_int(frame.pop()?)?;
                let a = expect_int(frame.pop()?)?;
                if !(a != b) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessNotEqLocals(a, b, t) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                if !(x != y) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessNotEqLocalConst(a, k, t) => {
                let x = local_int(&frame.locals, a)?;
                if !(x != k) {
                    frame.ip = t;
                }
            }
            OpCode::AddIntStore(a) => {
                let y = expect_int(frame.pop()?)?;
                let x = expect_int(frame.pop()?)?;
                frame.locals[a] = Value::Int(arith::add(x, y)?);
            }
            OpCode::SubIntStore(a) => {
                let y = expect_int(frame.pop()?)?;
                let x = expect_int(frame.pop()?)?;
                frame.locals[a] = Value::Int(arith::sub(x, y)?);
            }
            OpCode::MulIntStore(a) => {
                let y = expect_int(frame.pop()?)?;
                let x = expect_int(frame.pop()?)?;
                frame.locals[a] = Value::Int(arith::mul(x, y)?);
            }
            OpCode::AddFloatStore(a) => {
                let y = expect_float(frame.pop()?)?;
                let x = expect_float(frame.pop()?)?;
                frame.locals[a] = Value::Float(x + y);
            }
            OpCode::SubFloatStore(a) => {
                let y = expect_float(frame.pop()?)?;
                let x = expect_float(frame.pop()?)?;
                frame.locals[a] = Value::Float(x - y);
            }
            OpCode::MulFloatStore(a) => {
                let y = expect_float(frame.pop()?)?;
                let x = expect_float(frame.pop()?)?;
                frame.locals[a] = Value::Float(x * y);
            }
            OpCode::JumpUnlessLtConst(k, t) => {
                let x = expect_int(frame.pop()?)?;
                if !(x < k) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessGtConst(k, t) => {
                let x = expect_int(frame.pop()?)?;
                if !(x > k) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessLtEqConst(k, t) => {
                let x = expect_int(frame.pop()?)?;
                if !(x <= k) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessGtEqConst(k, t) => {
                let x = expect_int(frame.pop()?)?;
                if !(x >= k) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessEqConst(k, t) => {
                let x = expect_int(frame.pop()?)?;
                if !(x == k) {
                    frame.ip = t;
                }
            }
            OpCode::JumpUnlessNotEqConst(k, t) => {
                let x = expect_int(frame.pop()?)?;
                if !(x != k) {
                    frame.ip = t;
                }
            }
            OpCode::GetLocal2(a, b) => {
                let (x, y) = (frame.locals[a].clone(), frame.locals[b].clone());
                frame.push(x);
                frame.push(y);
            }
            OpCode::AddLocalsStore(dst, a, b) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                frame.locals[dst] = Value::Int(arith::add(x, y)?);
            }
            OpCode::SubLocalsStore(dst, a, b) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                frame.locals[dst] = Value::Int(arith::sub(x, y)?);
            }
            OpCode::MulLocalsStore(dst, a, b) => {
                let x = local_int(&frame.locals, a)?;
                let y = local_int(&frame.locals, b)?;
                frame.locals[dst] = Value::Int(arith::mul(x, y)?);
            }
            OpCode::FieldGetDup { local, field_index } => {
                let (s, v) = match &frame.locals[local] {
                    Value::Struct(s) => (s.clone(), s.borrow().1[field_index].clone()),
                    other => return Err(RuntimeError::TypeMismatch { expected: "struct", found: other.type_name() }),
                };
                frame.push(Value::Struct(s));
                frame.push(v);
            }
            OpCode::ReturnLocal(a) => {
                let v = frame.locals[a].clone();
                return Ok(Some(FrameOutcome::Returned(v)));
            }
            OpCode::IndexGetLocals(arr, i) => {
                let index = local_int(&frame.locals, i)?;
                let v = match &frame.locals[arr] {
                    Value::Array(a) => {
                        let items = a.borrow();
                        if index < 0 || index as usize >= items.len() {
                            return Err(RuntimeError::IndexOutOfBounds { index, len: items.len() });
                        }
                        items[index as usize].clone()
                    }
                    other => return Err(RuntimeError::TypeMismatch { expected: "array", found: other.type_name() }),
                };
                frame.push(v);
            }
            // fused array-index ops. bodies live out of line in `index_ops` so
            // they don't bloat this function (see that module's header)
            OpCode::IndexGetDupLocals(arr, i) => index_ops::get_dup_locals(&frame.locals, frame.value_stack, arr, i)?,
            OpCode::IndexAddConstLocals { arr, idx, k } => index_ops::add_const_locals(&frame.locals, arr, idx, k)?,
            OpCode::IndexAddLocalLocals { arr, idx, x } => index_ops::int_add(&frame.locals, arr, idx, x)?,
            OpCode::IndexSubLocalLocals { arr, idx, x } => index_ops::int_sub(&frame.locals, arr, idx, x)?,
            OpCode::IndexMulLocalLocals { arr, idx, x } => index_ops::int_mul(&frame.locals, arr, idx, x)?,
            OpCode::IndexAddFloatLocalLocals { arr, idx, x } => index_ops::float_add(&frame.locals, arr, idx, x)?,
            OpCode::IndexSubFloatLocalLocals { arr, idx, x } => index_ops::float_sub(&frame.locals, arr, idx, x)?,
            OpCode::IndexMulFloatLocalLocals { arr, idx, x } => index_ops::float_mul(&frame.locals, arr, idx, x)?,
            OpCode::IndexSetLocalsLocal { arr, idx, v } => index_ops::set_locals_local(&frame.locals, arr, idx, v)?,
            OpCode::IndexSetLocalsConst { arr, idx, c } => index_ops::set_locals_const(&frame.locals, &frame.chunk.constants, arr, idx, c)?,
            OpCode::IndexSetLocalLocal { idx, v } => index_ops::set_local_local(&frame.locals, frame.value_stack, idx, v)?,
            OpCode::IndexGetLocal(i) => {
                let index = local_int(&frame.locals, i)?;
                let array = expect_array(frame.pop()?)?;
                let items = array.borrow();
                if index < 0 || index as usize >= items.len() {
                    return Err(RuntimeError::IndexOutOfBounds { index, len: items.len() });
                }
                let v = items[index as usize].clone();
                drop(items);
                frame.push(v);
            }
            OpCode::FieldGetLocal { local, field_index } => {
                let v = match &frame.locals[local] {
                    Value::Struct(s) => s.borrow().1[field_index].clone(),
                    other => return Err(RuntimeError::TypeMismatch { expected: "struct", found: other.type_name() }),
                };
                frame.push(v);
            }

            OpCode::PushHandler { catch_target } => {
                let depth = frame.value_stack.len();
                frame.handlers.push((catch_target, depth));
            }
            OpCode::PopHandler => {
                frame.handlers.pop();
            }
            // the exception travels up as an `Err`. `run_chunk` of this frame (and then of
            // each caller) looks for a handler. program only dies if nobody catches it.
            OpCode::Throw => {
                let v = frame.pop()?;
                return Err(RuntimeError::UnhandledException(v.to_string()));
            }
            // fused struct-field ops. bodies out of line in `field_ops`
            OpCode::FieldAddConst { local, field_index, k } => field_ops::add_const(&frame.locals, local, field_index, k)?,
            OpCode::FieldIntAdd { local, field_index, x } => field_ops::int_add(&frame.locals, local, field_index, x)?,
            OpCode::FieldIntSub { local, field_index, x } => field_ops::int_sub(&frame.locals, local, field_index, x)?,
            OpCode::FieldIntMul { local, field_index, x } => field_ops::int_mul(&frame.locals, local, field_index, x)?,
            OpCode::FieldFloatAdd { local, field_index, x } => field_ops::float_add(&frame.locals, local, field_index, x)?,
            OpCode::FieldFloatSub { local, field_index, x } => field_ops::float_sub(&frame.locals, local, field_index, x)?,
            OpCode::FieldFloatMul { local, field_index, x } => field_ops::float_mul(&frame.locals, local, field_index, x)?,
            OpCode::FieldSetLocalLocal { local, field_index, v } => field_ops::set_local_local(&frame.locals, local, field_index, v)?,
            OpCode::FieldSetLocalConst { local, field_index, c } => field_ops::set_local_const(&frame.locals, &frame.chunk.constants, local, field_index, c)?,
        }

        Ok(None)
    }
}

#[inline(always)]
pub(crate) fn local_int(locals: &[Value], slot: usize) -> Result<i64, RuntimeError> {
    match &locals[slot] {
        Value::Int(n) => Ok(*n),
        // enums compare with the int opcodes (`==` / `!=` only, the checker won't allow more)
        Value::Enum(_, v) => Ok(*v as i64),
        other => Err(RuntimeError::TypeMismatch { expected: "int", found: other.type_name() }),
    }
}


#[inline(always)]
fn expect_int(v: Value) -> Result<i64, RuntimeError> {
    match v {
        Value::Int(n) => Ok(n),
        Value::Enum(_, v) => Ok(v as i64),
        other => Err(RuntimeError::TypeMismatch { expected: "int", found: other.type_name() }),
    }
}

#[inline(always)]
fn expect_float(v: Value) -> Result<f64, RuntimeError> {
    match v {
        Value::Float(n) => Ok(n),
        other => Err(RuntimeError::TypeMismatch { expected: "float", found: other.type_name() }),
    }
}

#[inline(always)]
fn expect_bool(v: Value) -> Result<bool, RuntimeError> {
    match v {
        Value::Bool(b) => Ok(b),
        other => Err(RuntimeError::TypeMismatch { expected: "bool", found: other.type_name() }),
    }
}

#[inline(always)]
fn expect_string(v: Value) -> Result<Rc<String>, RuntimeError> {
    match v {
        Value::String(s) => Ok(s),
        other => Err(RuntimeError::TypeMismatch { expected: "string", found: other.type_name() }),
    }
}

#[inline(always)]
fn expect_map(v: Value) -> Result<crate::value::MapRef, RuntimeError> {
    match v {
        Value::Map(m) => Ok(m),
        other => Err(RuntimeError::TypeMismatch { expected: "map", found: other.type_name() }),
    }
}

/// how a key looks in the "key not found" error. strings get quotes, rest as printed
fn key_text(key: &Value) -> String {
    match key {
        Value::String(s) => format!("\"{}\"", s),
        other => other.to_string(),
    }
}

fn expect_array(v: Value) -> Result<crate::value::ArrayRef, RuntimeError> {
    match v {
        Value::Array(a) => Ok(a),
        other => Err(RuntimeError::TypeMismatch { expected: "array", found: other.type_name() }),
    }
}

#[inline(always)]
fn expect_struct(v: Value) -> Result<crate::value::StructRef, RuntimeError> {
    match v {
        Value::Struct(s) => Ok(s),
        other => Err(RuntimeError::TypeMismatch { expected: "struct", found: other.type_name() }),
    }
}

/// pops two Ints (right-then-left, the compiler's order, fight me),
/// applies `f`, pushes the Int result. boring, but it works.
#[inline(always)]
fn binary_int(frame: &mut CallFrame, f: impl FnOnce(i64, i64) -> Result<i64, RuntimeError>) -> Result<(), RuntimeError> {
    let b = expect_int(frame.pop()?)?;
    let a = expect_int(frame.pop()?)?;
    frame.push(Value::Int(f(a, b)?));
    Ok(())
}

#[inline(always)]
fn binary_float(frame: &mut CallFrame, f: impl FnOnce(f64, f64) -> f64) -> Result<(), RuntimeError> {
    let b = expect_float(frame.pop()?)?;
    let a = expect_float(frame.pop()?)?;
    frame.push(Value::Float(f(a, b)));
    Ok(())
}

#[inline(always)]
fn binary_int_cmp(frame: &mut CallFrame, f: impl FnOnce(i64, i64) -> bool) -> Result<(), RuntimeError> {
    let b = expect_int(frame.pop()?)?;
    let a = expect_int(frame.pop()?)?;
    frame.push(Value::Bool(f(a, b)));
    Ok(())
}

/// fused `GetLocal(a); GetLocal(b); <CmpInt>`. reads both locals
/// directly (no stack round trip) and pushes the result.
#[inline(always)]
fn fused_int_cmp(frame: &mut CallFrame, a: usize, b: usize, f: impl FnOnce(i64, i64) -> bool) -> Result<(), RuntimeError> {
    let x = expect_int(frame.locals[a].clone())?;
    let y = expect_int(frame.locals[b].clone())?;
    frame.push(Value::Bool(f(x, y)));
    Ok(())
}

#[inline(always)]
fn binary_float_cmp(frame: &mut CallFrame, f: impl FnOnce(f64, f64) -> bool) -> Result<(), RuntimeError> {
    let b = expect_float(frame.pop()?)?;
    let a = expect_float(frame.pop()?)?;
    frame.push(Value::Bool(f(a, b)));
    Ok(())
}

// the float/string/bool fused helpers below read both locals directly,
// (like fused_int_cmp), so their order is f(locals[a],
// locals[b]) and NOT reversed like the pop-based binary_* helpers. careful.

#[inline(always)]
fn fused_int_arith(frame: &mut CallFrame, a: usize, b: usize, f: impl FnOnce(i64, i64) -> Result<i64, RuntimeError>) -> Result<(), RuntimeError> {
    let x = expect_int(frame.locals[a].clone())?;
    let y = expect_int(frame.locals[b].clone())?;
    frame.push(Value::Int(f(x, y)?));
    Ok(())
}

#[inline(always)]
fn fused_float_arith(frame: &mut CallFrame, a: usize, b: usize, f: impl FnOnce(f64, f64) -> f64) -> Result<(), RuntimeError> {
    let x = expect_float(frame.locals[a].clone())?;
    let y = expect_float(frame.locals[b].clone())?;
    frame.push(Value::Float(f(x, y)));
    Ok(())
}

#[inline(always)]
fn fused_float_cmp(frame: &mut CallFrame, a: usize, b: usize, f: impl FnOnce(f64, f64) -> bool) -> Result<(), RuntimeError> {
    let x = expect_float(frame.locals[a].clone())?;
    let y = expect_float(frame.locals[b].clone())?;
    frame.push(Value::Bool(f(x, y)));
    Ok(())
}

#[inline(always)]
fn fused_string_concat(frame: &mut CallFrame, a: usize, b: usize) -> Result<(), RuntimeError> {
    let x = expect_string(frame.locals[a].clone())?;
    let y = expect_string(frame.locals[b].clone())?;
    frame.push(Value::String(Rc::new(format!("{}{}", x, y))));
    Ok(())
}

#[inline(always)]
fn fused_string_cmp(frame: &mut CallFrame, a: usize, b: usize, f: impl FnOnce(&str, &str) -> bool) -> Result<(), RuntimeError> {
    let x = expect_string(frame.locals[a].clone())?;
    let y = expect_string(frame.locals[b].clone())?;
    frame.push(Value::Bool(f(&x, &y)));
    Ok(())
}

#[inline(always)]
fn fused_bool_cmp(frame: &mut CallFrame, a: usize, b: usize, f: impl FnOnce(bool, bool) -> bool) -> Result<(), RuntimeError> {
    let x = expect_bool(frame.locals[a].clone())?;
    let y = expect_bool(frame.locals[b].clone())?;
    frame.push(Value::Bool(f(x, y)));
    Ok(())
}
