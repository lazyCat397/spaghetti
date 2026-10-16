//! Bytecode format. Stack-based, not register-based — the simplest
//! model to emit recursively from a tree-shaped `TypedProgram`, and it
//! keeps the VM a plain loop over a `Vec<OpCode>`.
//!
//! A `Chunk` is one function's compiled body (plus an implicit "main"
//! chunk for top-level statements). Jumps are always instruction
//! indices within the same chunk; calls go through `Call` instead.

use std::rc::Rc;

use crate::value::Value;

/// `Copy` since every variant only holds small integers — that makes
/// cloning an instruction in the hot loop free instead of a real clone.
#[derive(Debug, Clone, Copy)]
pub enum OpCode {
    // ---- constants / literals ----
    /// push constants[idx] onto the stack
    Const(usize),
    PushVoid,

    // ---- locals ----
    /// push the value in local slot `idx`
    GetLocal(usize),
    /// pop the stack top into local slot `idx` (slot stays on the stack
    /// for `let x = ...` used as a statement — compiler pops it after)
    SetLocal(usize),

    // ---- globals ----
    /// push the value in global slot `idx`. Globals are every variable
    /// declared at a file's ROOT scope; unlike locals they live in one
    /// array shared by every frame, which is what lets a function read
    /// (and assign to) a top-level `let`/`const` without any closure
    /// machinery. Slots are assigned at compile time — never by name.
    GetGlobal(usize),
    /// store the stack top into global slot `idx`; like SetLocal it does
    /// NOT pop (the compiler emits the Pop right after)
    SetGlobal(usize),

    // ---- arithmetic (operands popped right-then-left, result pushed) ----
    AddInt,
    /// fused `GetLocal(a); GetLocal(b); AddInt` — same result, one
    /// opcode instead of three. Emitted by the peephole pass, not the
    /// main compiler (see spg-compiler::peephole).
    AddLocals(usize, usize),

    // rest of the arithmetic set, fused the same way as AddLocals —
    // division/modulo keep the same zero-check as their unfused form
    SubLocals(usize, usize),
    MulLocals(usize, usize),
    DivLocals(usize, usize),
    ModLocals(usize, usize),
    SubInt,
    MulInt,
    DivInt,
    ModInt,
    PowInt,
    AddFloat,
    SubFloat,
    MulFloat,
    DivFloat,
    ModFloat,
    PowFloat,
    ConcatString,
    NegInt,
    NegFloat,

    // ---- comparison ----
    EqInt,
    EqFloat,
    EqBool,
    EqString,
    NotEqInt,
    NotEqFloat,
    NotEqBool,
    NotEqString,
    LtInt,
    GtInt,
    LtEqInt,
    GtEqInt,
    LtFloat,
    GtFloat,
    LtEqFloat,
    GtEqFloat,

    // fused int comparisons, same idea as AddLocals — this is the shape
    // every `a < b`, `a > b`, etc. between two locals compiles to
    LtLocals(usize, usize),
    GtLocals(usize, usize),
    LtEqLocals(usize, usize),
    GtEqLocals(usize, usize),
    EqIntLocals(usize, usize),
    NotEqIntLocals(usize, usize),

    // same fusion, mirrored for float/string/bool. Bool has no ordering
    // so only Eq/NotEq exist for it; string only has concat and Eq/NotEq.
    AddFloatLocals(usize, usize),
    SubFloatLocals(usize, usize),
    MulFloatLocals(usize, usize),
    DivFloatLocals(usize, usize),
    ModFloatLocals(usize, usize),
    LtFloatLocals(usize, usize),
    GtFloatLocals(usize, usize),
    LtEqFloatLocals(usize, usize),
    GtEqFloatLocals(usize, usize),
    EqFloatLocals(usize, usize),
    NotEqFloatLocals(usize, usize),
    ConcatStringLocals(usize, usize),
    EqStringLocals(usize, usize),
    NotEqStringLocals(usize, usize),
    EqBoolLocals(usize, usize),
    NotEqBoolLocals(usize, usize),

    // ---- boolean ----
    Not,
    // no short-circuit-free And/Or — those compile to jumps (see below)

    // ---- enums / deep equality ----
    /// pop an enum value, push the literal its variant was declared with
    EnumValue,
    /// pop two values, push whether they are equal by CONTENT (arrays, maps, structs)
    EqValue,
    NotEqValue,

    // ---- maps ----
    /// pop `2 * pairs` values (pushed key, value, key, value, ...), push a new Map
    /// with those entries; a repeated key keeps the LAST value
    MakeMap(usize),
    /// pop key then map, push the value (runtime error if the key is missing)
    MapGet,
    /// pop value, then key, then map; `map[key] = value` in place. Pushes nothing.
    MapSet,

    // ---- arrays ----
    /// pop `count` values, push a new Array containing them in order
    MakeArray(usize),
    /// pop index then array, push array[index]
    IndexGet,
    /// pop value, then index, then array; mutate array[index] = value
    /// in place (Array is Rc<RefCell<>>, so no re-store back into a
    /// local is needed — same reason ArrayPush/ArraySort mutate in
    /// place instead of returning a new array). Pushes nothing: index
    /// assignment is compiled as a statement, not an expression.
    IndexSet,
    /// iterator setup/step for for-in — see spg-compiler for how these
    /// are sequenced; kept as two explicit ops so the VM never needs to
    /// inspect Value::Array internals outside these two spots
    /// pop array, push its length as Int
    ArrayLen,

    // ---- structs ----
    /// pop `field_count` values (in field-declaration order, same
    /// push-then-pop-in-reverse convention as MakeArray/Call), build a
    /// Value::Struct tagged with `Program.struct_defs[def_index]`, push
    /// it. `def_index` (not the struct's name) is what the compiler
    /// resolves at compile time — the VM never looks structs up by name.
    MakeStruct { def_index: usize, field_count: usize },
    /// pop a struct, push its field at position `field_index` (resolved
    /// from the field name at compile time — see spg-compiler)
    FieldGet { field_index: usize },
    /// pop a value then a struct; mutate the struct's field at
    /// `field_index` in place (Struct is Rc<RefCell<>>, same reasoning
    /// as IndexSet). Pushes nothing — field assignment is a statement.
    FieldSet { field_index: usize },

    // ---- control flow (absolute instruction indices in this chunk) ----
    Jump(usize),
    /// pop condition; jump if it was false
    JumpIfFalse(usize),
    /// `a ?? b`: when the stack top is not `none`, jump (the value stays as the result);
    /// when it is `none`, pop it and fall through to evaluate the default
    JumpIfNotNone(usize),

    // ---- functions ----
    /// call the function at Program.functions[func_index], consuming
    /// `argc` stack values as arguments and pushing its return value
    Call { func_index: usize, argc: usize },
    /// make a function value: pops `ncap` captured values (pushed in order) and
    /// pushes a closure over `Program.functions[func_index]`. `ncap: 0` is a named
    /// function used as a value
    MakeClosure { func_index: usize, ncap: usize },
    /// call a function VALUE: the stack holds the callee, then `argc` arguments.
    /// The callee's captures go before the arguments in its locals
    CallValue { argc: usize },
    /// fused `GetLocal(a); GetLocal(b); Call{argc:2}` — reads both
    /// locals directly instead of round-tripping through the stack
    CallLocals2 { func_index: usize, a: usize, b: usize },
    /// fused `GetLocal(a); Call{argc:1}`, the 1-arg counterpart above
    CallLocal1 { func_index: usize, a: usize },
    /// fused `GetLocal(a); Const(k); SubInt; Call{argc:1}` — the shape
    /// a decreasing recursive call like `fib(n - 1)` compiles to
    CallLocalMinusConst1 { func_index: usize, a: usize, k: i64 },
    /// pop the top of stack and return it from the current chunk
    Return,

    // ---- builtins ----
    /// generic builtin dispatch: pop `argc` values (reverse push order,
    /// same convention as `Call`), hand them to `Vm::call_builtin` keyed
    /// by `id`, push whatever it returns (`Value::Void` for builtins with
    /// no meaningful result). One opcode for every builtin instead of a
    /// dedicated opcode per builtin (the original `PrintBuiltin` shape) —
    /// keeps adding a builtin a signature-table + match-arm change, never
    /// a new opcode.
    CallBuiltin { id: spg_typecheck::BuiltinId, argc: usize },

    // ---- stack hygiene ----
    /// pop and discard the top of stack (statement-position expressions)
    Pop,
    /// duplicate the top stack value: `[.., a]` -> `[.., a, a]`. Used for
    /// `obj.field OP= v`: `obj` is pushed once, duplicated here so
    /// FieldGet can read the current field without consuming the object
    /// the following FieldSet still needs — same reasoning as Dup2, but
    /// only one value needs duplicating since the field is a compile-time
    /// constant (`field_index`), not a second stack value like an array
    /// index is.
    Dup1,
    /// duplicate the top two stack values as a pair, preserving order:
    /// `[.., a, b]` -> `[.., a, b, a, b]`. Used only for `arr[i] OP= v`:
    /// array+index are pushed once, duplicated here so IndexGet can read
    /// the current element without consuming the array/index the
    /// following IndexSet still needs — the alternative (compiling
    /// `array`/`index` a second time) would re-run any side effect in
    /// either expression twice.
    Dup2,

    // ---- exceptions (backs try/catch) ----
    /// establish a catch handler at instruction `catch_target` for the
    /// duration of this frame; unwound automatically by PopHandler
    PushHandler { catch_target: usize },
    PopHandler,
    /// pop a value and raise it as a runtime exception, unwinding to the
    /// nearest active handler (or aborting the program if none is active)
    Throw,

    // ---- superinstructions (emitted only by the peephole pass) ----
    /// fused `SetLocal(a); Pop` — moves the stack top into the slot
    StoreLocal(usize),
    /// fused `SetGlobal(a); Pop`
    StoreGlobal(usize),
    /// fused `GetLocal(a); Const(k); AddInt` (`SubInt` folds in as `-k`)
    AddLocalConst(usize, i64),
    /// fused `GetLocal(a); Const(k); MulInt`
    MulLocalConst(usize, i64),
    /// fused `GetLocal(a); Const(k); DivInt` — only when k != 0
    DivLocalConst(usize, i64),
    /// fused `GetLocal(a); Const(k); ModInt` — only when k != 0
    ModLocalConst(usize, i64),
    /// fused `AddLocalConst(a, k); StoreLocal(a)` — `a += k` in place
    IncLocal(usize, i64),
    /// fused `Const(k); AddInt` on the stack top (`SubInt` folds in as `-k`)
    AddConst(i64),
    MulConst(i64),
    DivConst(i64),
    ModConst(i64),
    LtLocalConst(usize, i64),
    GtLocalConst(usize, i64),
    LtEqLocalConst(usize, i64),
    GtEqLocalConst(usize, i64),
    EqLocalConst(usize, i64),
    NotEqLocalConst(usize, i64),
    /// fused `<int compare>; JumpIfFalse(t)` — pops two ints, jumps unless the
    /// relation holds (stack form, two-locals form, local-vs-constant form)
    JumpUnlessLt(usize),
    JumpUnlessLtLocals(usize, usize, usize),
    JumpUnlessLtLocalConst(usize, i64, usize),
    JumpUnlessGt(usize),
    JumpUnlessGtLocals(usize, usize, usize),
    JumpUnlessGtLocalConst(usize, i64, usize),
    JumpUnlessLtEq(usize),
    JumpUnlessLtEqLocals(usize, usize, usize),
    JumpUnlessLtEqLocalConst(usize, i64, usize),
    JumpUnlessGtEq(usize),
    JumpUnlessGtEqLocals(usize, usize, usize),
    JumpUnlessGtEqLocalConst(usize, i64, usize),
    JumpUnlessEq(usize),
    JumpUnlessEqLocals(usize, usize, usize),
    JumpUnlessEqLocalConst(usize, i64, usize),
    JumpUnlessNotEq(usize),
    JumpUnlessNotEqLocals(usize, usize, usize),
    JumpUnlessNotEqLocalConst(usize, i64, usize),
    /// fused `<Add|Sub|Mul>(Int|Float); StoreLocal(a)` — pops two operands and
    /// stores the result straight into the slot (`sum += expr`, `x = x * y`)
    AddIntStore(usize),
    SubIntStore(usize),
    MulIntStore(usize),
    AddFloatStore(usize),
    SubFloatStore(usize),
    MulFloatStore(usize),
    /// fused `Const(k); <int compare>; JumpIfFalse(t)` — compares the stack
    /// top with a constant and jumps unless the relation holds
    JumpUnlessLtConst(i64, usize),
    JumpUnlessGtConst(i64, usize),
    JumpUnlessLtEqConst(i64, usize),
    JumpUnlessGtEqConst(i64, usize),
    JumpUnlessEqConst(i64, usize),
    JumpUnlessNotEqConst(i64, usize),
    /// fused `GetLocal(a); GetLocal(b)` — pushes both
    GetLocal2(usize, usize),
    /// fused `<Add|Sub|Mul>Locals(a, b); StoreLocal(dst)` — `dst = a op b`
    AddLocalsStore(usize, usize, usize),
    SubLocalsStore(usize, usize, usize),
    MulLocalsStore(usize, usize, usize),
    /// fused `GetLocal(s); Dup1; FieldGet{field_index}` — pushes the struct
    /// and then its field (the head of every `s.f OP= v`)
    FieldGetDup { local: usize, field_index: usize },
    /// fused `GetLocal(a); Return`
    ReturnLocal(usize),
    /// fused `GetLocal(arr); GetLocal(i); IndexGet`
    IndexGetLocals(usize, usize),
    /// fused `GetLocal(i); IndexGet` — the array is already on the stack
    IndexGetLocal(usize),
    /// fused `GetLocal(arr); GetLocal(i); Dup2; IndexGet` — the head of every
    /// `arr[i] OP= v`: pushes the array, the index and the current element
    IndexGetDupLocals(usize, usize),
    /// fused `IndexGetDupLocals(arr, idx); AddConst(k); IndexSet` — `arr[idx] += k`
    /// in place (also covers `-= k`, which the optimizer already turns into a
    /// negated `AddConst`)
    IndexAddConstLocals { arr: usize, idx: usize, k: i64 },
    /// fused `IndexGetDupLocals(arr, idx); GetLocal(x); AddInt; IndexSet` — `arr[idx] += x`
    IndexAddLocalLocals { arr: usize, idx: usize, x: usize },
    /// same shape as `IndexAddLocalLocals` for `-=` / `*=` on ints and `+=` / `-=` / `*=` on
    /// floats: `IndexGetDupLocals(arr, idx); GetLocal(x); <op>; IndexSet`
    IndexSubLocalLocals { arr: usize, idx: usize, x: usize },
    IndexMulLocalLocals { arr: usize, idx: usize, x: usize },
    IndexAddFloatLocalLocals { arr: usize, idx: usize, x: usize },
    IndexSubFloatLocalLocals { arr: usize, idx: usize, x: usize },
    IndexMulFloatLocalLocals { arr: usize, idx: usize, x: usize },
    /// fused `GetLocal(arr); GetLocal(idx); GetLocal(v); IndexSet` — `arr[idx] = v`
    IndexSetLocalsLocal { arr: usize, idx: usize, v: usize },
    /// fused `GetLocal(arr); GetLocal(idx); Const(c); IndexSet` — `arr[idx] = <constant>`
    /// (`c` is a constant-pool index)
    IndexSetLocalsConst { arr: usize, idx: usize, c: usize },
    /// fused `GetLocal(idx); GetLocal(v); IndexSet` — the array is already on the
    /// stack (`m[i][j] = v`)
    IndexSetLocalLocal { idx: usize, v: usize },
    /// fused `GetLocal(s); FieldGet{field_index}`
    FieldGetLocal { local: usize, field_index: usize },

    // ---- fused struct-field ops (kept at the END of the enum on purpose) ----
    /// fused `FieldGetDup; AddConst(k); FieldSet` — `s.f += k` in place (also `-= k`,
    /// already a negated `AddConst`)
    FieldAddConst { local: usize, field_index: usize, k: i64 },
    /// fused `FieldGetDup; GetLocal(x); <op>; FieldSet` — `s.f OP= x` in place
    FieldIntAdd { local: usize, field_index: usize, x: usize },
    FieldIntSub { local: usize, field_index: usize, x: usize },
    FieldIntMul { local: usize, field_index: usize, x: usize },
    FieldFloatAdd { local: usize, field_index: usize, x: usize },
    FieldFloatSub { local: usize, field_index: usize, x: usize },
    FieldFloatMul { local: usize, field_index: usize, x: usize },
    /// fused `GetLocal(s); GetLocal(v); FieldSet` — `s.f = v`
    FieldSetLocalLocal { local: usize, field_index: usize, v: usize },
    /// fused `GetLocal(s); Const(c); FieldSet` — `s.f = <constant>` (`c` indexes the constant pool)
    FieldSetLocalConst { local: usize, field_index: usize, c: usize },
}

impl OpCode {
    /// the instruction index this op can jump to, if it is a jump-like op.
    /// The peephole pass uses this to know which instructions are jump
    /// targets (an instruction that is a target must never be folded into
    /// the middle of a fused op) and to rewrite targets after folding.
    pub fn jump_target(&self) -> Option<usize> {
        match *self {
            OpCode::Jump(t) | OpCode::JumpIfFalse(t) | OpCode::JumpIfNotNone(t) => Some(t),
            OpCode::JumpUnlessLtConst(_, t) | OpCode::JumpUnlessGtConst(_, t) | OpCode::JumpUnlessLtEqConst(_, t) | OpCode::JumpUnlessGtEqConst(_, t) | OpCode::JumpUnlessEqConst(_, t) | OpCode::JumpUnlessNotEqConst(_, t) | OpCode::JumpUnlessLt(t) | OpCode::JumpUnlessLtLocals(_, _, t) | OpCode::JumpUnlessLtLocalConst(_, _, t) | OpCode::JumpUnlessGt(t) | OpCode::JumpUnlessGtLocals(_, _, t) | OpCode::JumpUnlessGtLocalConst(_, _, t) | OpCode::JumpUnlessLtEq(t) | OpCode::JumpUnlessLtEqLocals(_, _, t) | OpCode::JumpUnlessLtEqLocalConst(_, _, t) | OpCode::JumpUnlessGtEq(t) | OpCode::JumpUnlessGtEqLocals(_, _, t) | OpCode::JumpUnlessGtEqLocalConst(_, _, t) | OpCode::JumpUnlessEq(t) | OpCode::JumpUnlessEqLocals(_, _, t) | OpCode::JumpUnlessEqLocalConst(_, _, t) | OpCode::JumpUnlessNotEq(t) | OpCode::JumpUnlessNotEqLocals(_, _, t) | OpCode::JumpUnlessNotEqLocalConst(_, _, t) => Some(t),
            OpCode::PushHandler { catch_target } => Some(catch_target),
            _ => None,
        }
    }

    pub fn jump_target_mut(&mut self) -> Option<&mut usize> {
        match self {
            OpCode::Jump(t) | OpCode::JumpIfFalse(t) | OpCode::JumpIfNotNone(t) => Some(t),
            OpCode::JumpUnlessLtConst(_, t) | OpCode::JumpUnlessGtConst(_, t) | OpCode::JumpUnlessLtEqConst(_, t) | OpCode::JumpUnlessGtEqConst(_, t) | OpCode::JumpUnlessEqConst(_, t) | OpCode::JumpUnlessNotEqConst(_, t) | OpCode::JumpUnlessLt(t) | OpCode::JumpUnlessLtLocals(_, _, t) | OpCode::JumpUnlessLtLocalConst(_, _, t) | OpCode::JumpUnlessGt(t) | OpCode::JumpUnlessGtLocals(_, _, t) | OpCode::JumpUnlessGtLocalConst(_, _, t) | OpCode::JumpUnlessLtEq(t) | OpCode::JumpUnlessLtEqLocals(_, _, t) | OpCode::JumpUnlessLtEqLocalConst(_, _, t) | OpCode::JumpUnlessGtEq(t) | OpCode::JumpUnlessGtEqLocals(_, _, t) | OpCode::JumpUnlessGtEqLocalConst(_, _, t) | OpCode::JumpUnlessEq(t) | OpCode::JumpUnlessEqLocals(_, _, t) | OpCode::JumpUnlessEqLocalConst(_, _, t) | OpCode::JumpUnlessNotEq(t) | OpCode::JumpUnlessNotEqLocals(_, _, t) | OpCode::JumpUnlessNotEqLocalConst(_, _, t) => Some(t),
            OpCode::PushHandler { catch_target } => Some(catch_target),
            _ => None,
        }
    }
}

/// where an instruction came from: the source file (when known) and line.
/// Shown in stack traces; the VM never reads it on the normal path.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loc {
    pub file: Option<std::rc::Rc<str>>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub name: String,
    pub code: Vec<OpCode>,
    /// one entry per instruction in `code`, same order
    pub locs: Vec<Loc>,
    /// location given to every instruction emitted from now on
    pub cur_loc: Loc,
    pub constants: Vec<Value>,
    /// number of local variable slots this chunk needs (params + lets),
    /// allocated up front per call frame
    pub num_locals: usize,
    pub arity: usize,
    /// parameter and return kinds, when all of them are `int`, `float` or `bool`
    /// (set by the compiler). Only these functions can run as native code (spg-jit).
    pub native_sig: Option<(Vec<NativeKind>, NativeKind)>,
}

impl Chunk {
    pub fn new(name: impl Into<String>) -> Self {
        Chunk {
            name: name.into(),
            code: Vec::new(),
            locs: Vec::new(),
            cur_loc: Loc::default(),
            constants: Vec::new(),
            num_locals: 0,
            arity: 0,
            native_sig: None,
        }
    }

    /// appends an instruction, returning its index (used by the compiler
    /// to patch jump targets after the fact — emit a placeholder Jump,
    /// remember the index, backpatch once the real target is known)
    pub fn emit(&mut self, op: OpCode) -> usize {
        self.code.push(op);
        self.locs.push(self.cur_loc.clone());
        self.code.len() - 1
    }

    pub fn add_constant(&mut self, value: Value) -> usize {
        self.constants.push(value);
        self.constants.len() - 1
    }

    pub fn patch_jump(&mut self, index: usize, target: usize) {
        match &mut self.code[index] {
            OpCode::Jump(t) | OpCode::JumpIfFalse(t) | OpCode::JumpIfNotNone(t) => *t = target,
            OpCode::PushHandler { catch_target } => *catch_target = target,
            other => panic!("patch_jump called on non-jump op: {:?}", other),
        }
    }

    /// index the next emitted instruction will get — used to compute
    /// jump targets (e.g. "jump back to the top of this loop")
    pub fn next_index(&self) -> usize {
        self.code.len()
    }
}

/// A whole compiled program: one chunk per user function, plus an
/// implicit chunk for top-level statements (`main`). Function calls
/// reference other chunks by index into `functions`.
#[derive(Debug, Clone)]
pub struct Program {
    pub main: Chunk,
    pub functions: Vec<Chunk>,
    /// function name -> index into `functions`, so the VM/compiler can
    /// resolve a call by name without a linear scan
    pub function_names: Vec<String>,
    /// struct definitions, indexed by `OpCode::MakeStruct`'s `def_index`
    /// — shared (`Rc`) so every `Value::Struct` built from the same
    /// struct type points at the same `StructDef` instead of cloning its
    /// name/field-name strings per instance
    pub struct_defs: Vec<Rc<crate::value::StructDef>>,
    /// enum definitions, indexed by `Value::Enum`'s first field (only used to
    /// print a variant's name; comparing enums never looks at them)
    pub enum_defs: Vec<Rc<crate::value::EnumDef>>,
    /// how many global slots the VM must allocate before running `main`
    pub num_globals: usize,
}

/// a value kind the native backend understands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    Int,
    Float,
    Bool,
    /// a reference-counted string; native code owns one reference per value it holds
    String,
    /// an array whose elements are all of this kind (arrays are reference counted too)
    Array(ElemKind),
    /// a struct value of the struct type `Program.struct_defs[index]`
    Struct(usize),
}

/// element kinds an array can have in native code (nested arrays are not supported)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemKind {
    Int,
    Float,
    Bool,
    String,
}

impl ElemKind {
    /// the kind of one element, as a value on the native stack
    pub fn value_kind(self) -> NativeKind {
        match self {
            ElemKind::Int => NativeKind::Int,
            ElemKind::Float => NativeKind::Float,
            ElemKind::Bool => NativeKind::Bool,
            ElemKind::String => NativeKind::String,
        }
    }
}
