//! The bytecode interpreter — a plain loop over a `Vec<OpCode>`, fully
//! synchronous. If I ever need real concurrency I'll reach for
//! `std::thread` rather than build it into the VM.

use std::rc::Rc;

use crate::builtins::call_builtin_on_stack;
use crate::field_ops;
use crate::index_ops;
use crate::chunk::{Chunk, OpCode, Program};
use crate::arith;
use crate::error::RuntimeError;
use crate::value::{Closure, Value};

/// One active function call. Locals live in their own `Vec` per frame,
/// but the operand stack is a single `Vec<Value>` shared across the
/// whole call chain and borrowed here by reference — that avoids
/// allocating a fresh stack on every call.
struct CallFrame<'p, 'v> {
    chunk: &'p Chunk,
    ip: usize,
    locals: Vec<Value>,
    /// shared across the whole call chain; a frame only ever pops what
    /// it (or its own callees) pushed, so the sharing stays safe
    value_stack: &'v mut Vec<Value>,
    /// active `try` handlers in this frame, innermost last — each entry is
    /// `(catch block's instruction index, operand-stack height when the try
    /// began)`. The height lets an exception that arrives in the middle of an
    /// expression (a callee threw) drop the half-built temporaries first.
    /// An exception nobody in this frame handles leaves the frame as an
    /// `Err`, and the caller's `run_chunk` looks in ITS handlers: that is
    /// how a `throw` reaches a `try` in a calling function.
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
    /// recycled `Vec`s for call frames' locals: a call takes one (already
    /// allocated) instead of hitting the allocator, and gives it back on return
    pool: Vec<Vec<Value>>,
    /// every root-scope variable of the whole program, indexed by the
    /// slot the compiler assigned. One array for the entire run, shared
    /// by every call frame — see OpCode::GetGlobal.
    globals: Vec<Value>,
    /// the call chain of the error being propagated, innermost frame first.
    /// Filled while the error unwinds, cleared again when a `try` catches it.
    trace: Vec<TraceFrame>,
    /// frames that did not fit in `trace` (a very deep recursion keeps only
    /// its innermost frames; the count says how many were left out)
    trace_omitted: usize,
    /// how many frames are running right now (every spg call is one native recursion)
    depth: usize,
}

/// deepest spg call chain allowed. The CLI runs the VM on a 1 GiB thread stack,
/// which holds well over this many frames; the limit turns an endless recursion
/// into a catchable error with a trace instead of a process abort
pub const MAX_CALL_DEPTH: usize = 100_000;

/// how many frames a stack trace keeps before it starts counting instead
const TRACE_LIMIT: usize = 50;

/// one line of a stack trace: the function, and where in the source it stopped
#[derive(Debug, Clone, PartialEq)]
pub struct TraceFrame {
    /// `<main>` for the top-level code, otherwise the function's name
    pub function: String,
    pub file: Option<String>,
    pub line: u32,
}

/// what one step of execution produced — right now that's only "the
/// frame returned with this value"
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
        }
    }

    /// Runs the program's `main` chunk to completion and returns
    /// whatever it left on top of the stack (always `Value::Void` in
    /// practice, since `main` has no way to produce a value in spg —
    /// kept as a real return in case that changes).
    ///
    /// The operand stack is a single `Vec<Value>` shared across every
    /// call in the run, rather than one per frame — that's what lets a
    /// function call skip allocating its own stack. I tried this and a
    /// few other approaches (an explicit call stack instead of native
    /// recursion, shrinking `Value`) in SpgLab, a throwaway copy of the
    /// VM used for testing performance ideas before they land here.
    pub fn run(&mut self) -> Result<Value, RuntimeError> {
        self.trace.clear();
        self.trace_omitted = 0;
        let mut value_stack: Vec<Value> = Vec::with_capacity(256);
        let result = self.run_chunk(&self.program.main, Vec::new(), &mut value_stack);
        // buffered `print` output must land before the caller reports anything
        crate::builtins::flush_stdout();
        result
    }

    /// Runs one chunk (a function body or `main`) to completion, given
    /// its already-evaluated arguments. Nested spg calls recurse into
    /// this function again, so a pathologically deep spg recursion will
    /// eventually hit a Rust stack overflow rather than a clean spg-level
    /// error — a deliberate simplification, not a temporary one.
    /// `value_stack` is threaded through by reference instead of living
    /// on `Vm` itself, which avoids a borrow conflict on the recursive
    /// call.
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
                    // caught here: whatever the deeper frames recorded belonged to
                    // an error that is now handled, so it is not a trace anymore
                    self.trace.clear();
                    self.trace_omitted = 0;
                }
            }
        }
    }

    /// adds this frame to the trace of the error that is leaving it
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
    /// many frames were left out. Empty after a successful run.
    pub fn take_trace(&mut self) -> (Vec<TraceFrame>, usize) {
        let omitted = std::mem::take(&mut self.trace_omitted);
        (std::mem::take(&mut self.trace), omitted)
    }

    /// An error surfaced in `frame`. If it is catchable (a `throw`, or a runtime
    /// condition like a division by zero or a missing map key) and this frame
    /// has an active `try`, jump to its catch block with the message on the stack;
    /// otherwise hand the error up (the caller's frame gets the same chance).
    /// VM bugs are never catchable. Kept out of the hot loop on purpose.
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

    /// Executes one instruction. Returns `Some(Returned(v))` once
    /// `Return` fires, `None` otherwise. `ip` advances by one each call
    /// except for jumps, which set it directly.
    #[inline(always)]
    fn step(&mut self, frame: &mut CallFrame<'p, '_>) -> Result<Option<FrameOutcome>, RuntimeError> {
        let op = frame.chunk.code[frame.ip];
        frame.ip += 1;

        match op {
            OpCode::Const(idx) => frame.push(frame.chunk.constants[idx].clone()),
            OpCode::PushVoid => frame.push(Value::Void),

            OpCode::GetGlobal(slot) => frame.push(self.globals[slot].clone()),
            OpCode::SetGlobal(slot) => {
                // same no-pop contract as SetLocal
                let v = frame.value_stack.last().ok_or(RuntimeError::StackUnderflow)?.clone();
                self.globals[slot] = v;
            }

            OpCode::GetLocal(slot) => frame.push(frame.locals[slot].clone()),
            OpCode::SetLocal(slot) => {
                // doesn't pop — the compiler always emits a Pop right
                // after, once the assignment's own value has served its
                // purpose. Leaving it on the stack here keeps room for a
                // future "assignment as expression" without changing
                // this opcode's format.
                let v = frame.value_stack.last().ok_or(RuntimeError::StackUnderflow)?.clone();
                frame.locals[slot] = v;
            }

            // arithmetic: operands pop right-then-left, since the
            // compiler pushes left then right
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
            // the typechecker only allows int**int with a non-mixed type, so a
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

            // ---- comparison ----
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

            // fused GetLocal(a); GetLocal(b); <CmpInt> — same idea as
            // AddLocals above, extended to comparisons since they're the
            // backbone of most if/while conditions
            OpCode::LtLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x < y)?,
            OpCode::GtLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x > y)?,
            OpCode::LtEqLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x <= y)?,
            OpCode::GtEqLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x >= y)?,
            OpCode::EqIntLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x == y)?,
            OpCode::NotEqIntLocals(a, b) => fused_int_cmp(frame, a, b, |x, y| x != y)?,

            // same fusion, mirrored for float/string/bool
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
                let callee =
                    self.program.functions.get(func_index).ok_or(RuntimeError::InvalidFunctionIndex(func_index))?;
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.reserve(callee.num_locals);
                // the arguments are the top `argc` stack values, already in order
                let at = frame.value_stack.len() - argc;
                locals.extend(frame.value_stack.drain(at..));
                let result = self.run_chunk(callee, locals, frame.value_stack)?;
                frame.push(result);
            }
            OpCode::MakeClosure { func_index, ncap } => {
                // the captured values are the top `ncap` stack values, in order
                let at = frame.value_stack.len() - ncap;
                let captures: Vec<Value> = frame.value_stack.drain(at..).collect();
                frame.push(Value::Func(Rc::new(Closure { index: func_index, captures })));
            }
            OpCode::CallValue { argc } => {
                // stack: callee, arg1 .. argN (callee below the arguments)
                let at = frame.value_stack.len() - argc;
                let mut locals = self.pool.pop().unwrap_or_default();
                locals.extend(frame.value_stack.drain(at..));
                let callee_value = frame.pop()?;
                let Value::Func(closure) = callee_value else {
                    return Err(RuntimeError::TypeMismatch { expected: "function", found: callee_value.type_name() });
                };
                // the captured values go before the arguments
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
                // same semantics as a plain SubInt, just fused into the call
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

            // ---- superinstructions ----
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
            // fused array-index ops: bodies live out of line in `index_ops` so
            // they do not grow this function (see that module's header)
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
            // the exception travels up as an `Err`: `run_chunk` of this frame (and then of
            // each caller) looks for a handler, and the program ends only if none is found
            OpCode::Throw => {
                let v = frame.pop()?;
                return Err(RuntimeError::UnhandledException(v.to_string()));
            }
            // fused struct-field ops: bodies out of line in `field_ops`
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
        // enums compare with the int opcodes (`==` / `!=` only; the checker allows nothing else)
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

/// how a key reads in the "key not found" error: strings quoted, the rest as printed
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

/// pops two Ints (right-then-left, per the compiler's push order),
/// applies `f`, pushes the Int result
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

/// fused `GetLocal(a); GetLocal(b); <CmpInt>` — reads both locals
/// directly (skipping the stack round-trip) and pushes the result
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

// the float/string/bool fused helpers below read both locals directly
// (like fused_int_cmp), so their operand order is f(locals[a],
// locals[b]) rather than reversed like the pop-based binary_* helpers

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
