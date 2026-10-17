//! Bytecode format. Stack-based, not register-based. simplest thing
//! to emit recursively from a tree-shaped `TypedProgram`. and it
//! keeps the VM a dumb loop over a `Vec<OpCode>`. that's the whole point.
//!
//! A `Chunk` is one function's compiled body (plus the implicit "main"
//! chunk for top-level stuff). Jumps are always instruction
//! indices inside the same chunk. calls go through `Call`. don't mix them.

use std::rc::Rc;

use crate::value::Value;

/// `Copy` since every variant only holds small integers. that makes
/// cloning an instruction in the hot loop free. no real clone, no drama.
#[derive(Debug, Clone, Copy)]
pub enum OpCode {
    // ---- constants / literals (the boring part) ----
    /// push constants[idx]. nothing clever.
    Const(usize),
    PushVoid,

    // ---- locals (where the real work lives) ----
    /// push the value in local slot `idx`. good luck finding it.
    GetLocal(usize),
    /// pop the stack top into local slot `idx` (slot stays on the stack,
    /// for `let x = ...` as a statement. compiler pops it after. messy, works.
    SetLocal(usize),

    // ---- globals (yes, we have these. no, I'm not proud) ----
    /// push the value in global slot `idx`. globals are every variable
    /// declared at a file's ROOT scope. unlike locals they live in one
    /// array shared by every frame. that's how a function can read
    /// (and assign to) a top-level `let`/`const` with zero closure
    /// machinery. slots get assigned at compile time. never by name. ever.
    GetGlobal(usize),
    /// store the stack top into global slot `idx`. like SetLocal it does
    /// NOT pop (the compiler emits the Pop right after, every time)
    SetGlobal(usize),

    // ---- arithmetic (right-then-left pops, because reasons) ----
    AddInt,
    /// fused `GetLocal(a); GetLocal(b); AddInt`. same result, one
    /// opcode instead of three. the peephole pass makes these, not the
    /// main compiler (see spg-compiler::peephole). sneaky little optimizer.
    AddLocals(usize, usize),

    // rest of the arithmetic set, fused the same way as AddLocals.
    // div/mod keep the zero check. don't you dare remove it.
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

    // ---- comparison (the fun part) ----
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

    // fused int comparisons, same idea as AddLocals. this is the shape
    // every `a < b`, `a > b`, etc. between two locals becomes
    LtLocals(usize, usize),
    GtLocals(usize, usize),
    LtEqLocals(usize, usize),
    GtEqLocals(usize, usize),
    EqIntLocals(usize, usize),
    NotEqIntLocals(usize, usize),

    // same fusion, mirrored for float/string/bool. bool has no ordering,
    // so only Eq/NotEq for it. string only has concat and Eq/NotEq. that's it.
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

    // ---- boolean (tiny, like my social life) ----
    Not,
    // no And/Or opcodes. those compile to jumps (see below). jumps are fun.

    // ---- enums / deep equality (ugh) ----
    /// pop an enum value, push the literal its variant was declared with.
    EnumValue,
    /// pop two values, push if they're equal by CONTENT (arrays, maps, structs). slow, probably.
    EqValue,
    NotEqValue,

    // ---- maps (a hashmap in a trenchcoat) ----
    /// pop `2 * pairs` values (key, value, key, value...), push a new Map
    /// with those entries. repeated key? last one wins. deal with it.
    MakeMap(usize),
    /// pop key then map, push the value. key missing? runtime error. good luck.
    MapGet,
    /// pop value, key, map. `map[key] = value` in place. pushes nothing.
    MapSet,

    // ---- arrays (the Rc<RefCell> saga) ----
    /// pop `count` values, push a new Array with them. in order. yes.
    MakeArray(usize),
    /// pop index then array, push array[index]. no bounds joke here, sorry.
    IndexGet,
    /// pop value, index, array; mutate array[index] = value
    /// in place (Array is Rc<RefCell<>>, so no storing it back into a
    /// local. same reason ArrayPush/ArraySort mutate in
    /// place instead of returning a new array). pushes nothing: index
    /// assignment is a statement, not an expression. don't get clever.
    IndexSet,
    /// iterator setup/step for for-in. see spg-compiler for how these
    /// are sequenced. kept as two ops so the VM never has to
    /// poke at Value::Array internals outside these two spots
    /// pop array, push its length as Int. boring, important.
    ArrayLen,

    // ---- structs (plain old data with trust issues) ----
    /// pop `field_count` values (in field-declaration order, same
    /// push-then-pop-in-reverse thing as MakeArray/Call), build a
    /// Value::Struct tagged with `Program.struct_defs[def_index]`, push
    /// it. `def_index` (not the name) is what the compiler
    /// resolves at compile time. the VM never looks structs up by name.
    MakeStruct { def_index: usize, field_count: usize },
    /// pop a struct, push its field at `field_index` (resolved
    /// from the field name at compile time. see spg-compiler).
    FieldGet { field_index: usize },
    /// pop a value then a struct; mutate the field at
    /// `field_index` in place (Struct is Rc<RefCell<>>, same reasoning
    /// as IndexSet). pushes nothing. field assignment is a statement, fine.
    FieldSet { field_index: usize },

    // ---- control flow (absolute indices, don't touch the numbers) ----
    Jump(usize),
    /// pop condition; jump if false. the classic.
    JumpIfFalse(usize),
    /// `a ?? b`: if the top isn't `none`, jump (value stays as the result).
    /// if it IS `none`, pop it and fall through to the default. cute.
    JumpIfNotNone(usize),

    // ---- functions (where the stack gets crowded) ----
    /// call the function at Program.functions[func_index], eating
    /// `argc` stack values as args and pushing its return value.
    Call { func_index: usize, argc: usize },
    /// make a function value: pops `ncap` captured values (in order) and
    /// pushes a closure over `Program.functions[func_index]`. `ncap: 0` = named
    /// function used as a value. fancy.
    MakeClosure { func_index: usize, ncap: usize },
    /// call a function VALUE: the stack holds the callee, then `argc` args.
    /// the callee's captures go before the args in its locals.
    CallValue { argc: usize },
    /// fused `GetLocal(a); GetLocal(b); Call{argc:2}`. reads both
    /// locals directly instead of a stack round trip. mild speed boost.
    CallLocals2 { func_index: usize, a: usize, b: usize },
    /// fused `GetLocal(a); Call{argc:1}`. the 1-arg cousin of the above.
    CallLocal1 { func_index: usize, a: usize },
    /// fused `GetLocal(a); Const(k); SubInt; Call{argc:1}`. the shape
    /// a recursive call like `fib(n - 1)` turns into. fib lives here.
    CallLocalMinusConst1 { func_index: usize, a: usize, k: i64 },
    /// pop the top of stack and return it from this chunk. bye.
    Return,

    // ---- builtins (the fast lane to Rust) ----
    /// generic builtin dispatch: pop `argc` values (reverse order,
    /// same as `Call`), hand them to `Vm::call_builtin` keyed
    /// by `id`, push whatever it returns (`Value::Void` for builtins that
    /// return nothing useful). one opcode for every builtin instead of one
    /// per builtin (the original `PrintBuiltin` shape, RIP).
    /// adding a builtin = a table entry + a match arm. never a new opcode.
    /// (you're welcome, future me)
    CallBuiltin { id: spg_typecheck::BuiltinId, argc: usize },

    // ---- stack hygiene (clean up after yourself) ----
    /// pop and throw away the top (expressions as statements). yeet.
    Pop,
    /// duplicate the top value: `[.., a]` -> `[.., a, a]`. used for
    /// `obj.field OP= v`. `obj` is pushed once, duplicated here so
    /// FieldGet can read the field without eating the object
    /// FieldSet still needs. same reasoning as Dup2, but
    /// only one value needs it since the field index is compile-time
    /// (`field_index`), not a second stack value like an array
    /// index is. lucky us.
    Dup1,
    /// duplicate the top two values as a pair, order kept:
    /// `[.., a, b]` -> `[.., a, b, a, b]`. only for `arr[i] OP= v`.
    /// array+index get pushed once, duplicated here so IndexGet can read
    /// the element without eating the array/index the
    /// IndexSet still needs. the alternative (compiling
    /// `array`/`index` twice) would re-run side effects. nope.
    /// either expression twice. nobody wants that.
    Dup2,

    // ---- exceptions (try/catch lives here) ----
    /// set up a catch handler at `catch_target` for the
    /// duration of this frame. PopHandler tears it down.
    PushHandler { catch_target: usize },
    PopHandler,
    /// pop a value and throw it as a runtime exception, unwinding to the
    /// nearest handler (or the program dies if there isn't one). rip.
    Throw,

    // ---- superinstructions (the peephole pass is a menace) ----
    /// fused `SetLocal(a); Pop`. moves the top into the slot.
    StoreLocal(usize),
    /// fused `SetGlobal(a); Pop`. same deal, for globals.
    StoreGlobal(usize),
    /// fused `GetLocal(a); Const(k); AddInt` (`SubInt` folds in as `-k`).
    AddLocalConst(usize, i64),
    /// fused `GetLocal(a); Const(k); MulInt`. multiply, you know the drill.
    MulLocalConst(usize, i64),
    /// fused `GetLocal(a); Const(k); DivInt`. only when k != 0. obviously.
    DivLocalConst(usize, i64),
    /// fused `GetLocal(a); Const(k); ModInt`. only when k != 0. same as above.
    ModLocalConst(usize, i64),
    /// fused `AddLocalConst(a, k); StoreLocal(a)`. `a += k` in place.
    IncLocal(usize, i64),
    /// fused `Const(k); AddInt` on the top (`SubInt` folds in as `-k`).
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
    /// fused `<int compare>; JumpIfFalse(t)`. pops two ints, jumps unless the
    /// relation holds (stack form, two-locals form, local-vs-const form). ok.
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
    /// fused `<Add|Sub|Mul>(Int|Float); StoreLocal(a)`. pops two and
    /// stores the result straight into the slot (`sum += expr`, `x = x * y`).
    AddIntStore(usize),
    SubIntStore(usize),
    MulIntStore(usize),
    AddFloatStore(usize),
    SubFloatStore(usize),
    MulFloatStore(usize),
    /// fused `Const(k); <int compare>; JumpIfFalse(t)`. compares the top
    /// with a constant and jumps unless the relation holds.
    JumpUnlessLtConst(i64, usize),
    JumpUnlessGtConst(i64, usize),
    JumpUnlessLtEqConst(i64, usize),
    JumpUnlessGtEqConst(i64, usize),
    JumpUnlessEqConst(i64, usize),
    JumpUnlessNotEqConst(i64, usize),
    /// fused `GetLocal(a); GetLocal(b)`. pushes both. two for one.
    GetLocal2(usize, usize),
    /// fused `<Add|Sub|Mul>Locals(a, b); StoreLocal(dst)`. `dst = a op b`.
    AddLocalsStore(usize, usize, usize),
    SubLocalsStore(usize, usize, usize),
    MulLocalsStore(usize, usize, usize),
    /// fused `GetLocal(s); Dup1; FieldGet{field_index}`. pushes the struct
    /// and then its field (the head of every `s.f OP= v`).
    FieldGetDup { local: usize, field_index: usize },
    /// fused `GetLocal(a); Return`. the lazy return.
    ReturnLocal(usize),
    /// fused `GetLocal(arr); GetLocal(i); IndexGet`. array access, no frills.
    IndexGetLocals(usize, usize),
    /// fused `GetLocal(i); IndexGet`. the array is already on the stack.
    IndexGetLocal(usize),
    /// fused `GetLocal(arr); GetLocal(i); Dup2; IndexGet`. the head of every
    /// `arr[i] OP= v`. pushes the array, the index and the current element.
    IndexGetDupLocals(usize, usize),
    /// fused `IndexGetDupLocals(arr, idx); AddConst(k); IndexSet`. `arr[idx] += k`
    /// in place (also `-= k`, the optimizer already flips it to a
    /// negated `AddConst`). ok, optimizer, you win this one.
    IndexAddConstLocals { arr: usize, idx: usize, k: i64 },
    /// fused `IndexGetDupLocals(arr, idx); GetLocal(x); AddInt; IndexSet`. `arr[idx] += x`
    IndexAddLocalLocals { arr: usize, idx: usize, x: usize },
    /// same shape as `IndexAddLocalLocals`, for `-=` / `*=` on ints and `+=` / `-=` / `*=` on
    /// floats. `IndexGetDupLocals(arr, idx); GetLocal(x); <op>; IndexSet`. copy paste, yes.
    IndexSubLocalLocals { arr: usize, idx: usize, x: usize },
    IndexMulLocalLocals { arr: usize, idx: usize, x: usize },
    IndexAddFloatLocalLocals { arr: usize, idx: usize, x: usize },
    IndexSubFloatLocalLocals { arr: usize, idx: usize, x: usize },
    IndexMulFloatLocalLocals { arr: usize, idx: usize, x: usize },
    /// fused `GetLocal(arr); GetLocal(idx); GetLocal(v); IndexSet`. `arr[idx] = v`.
    IndexSetLocalsLocal { arr: usize, idx: usize, v: usize },
    /// fused `GetLocal(arr); GetLocal(idx); Const(c); IndexSet`. `arr[idx] = <const>`.
    /// (`c` is a const pool index. the numbers are the point)
    IndexSetLocalsConst { arr: usize, idx: usize, c: usize },
    /// fused `GetLocal(idx); GetLocal(v); IndexSet`. the array is already on the
    /// stack (`m[i][j] = v`). nested arrays, the classic pain.
    IndexSetLocalLocal { idx: usize, v: usize },
    /// fused `GetLocal(s); FieldGet{field_index}`. a field, quick.
    FieldGetLocal { local: usize, field_index: usize },

    // ---- fused struct-field ops (at the END of the enum on purpose, DO NOT MOVE THEM) ----
    /// fused `FieldGetDup; AddConst(k); FieldSet`. `s.f += k` in place (also `-= k`,
    /// already a negated `AddConst`). the optimizer is smug about this.
    FieldAddConst { local: usize, field_index: usize, k: i64 },
    /// fused `FieldGetDup; GetLocal(x); <op>; FieldSet`. `s.f OP= x` in place.
    FieldIntAdd { local: usize, field_index: usize, x: usize },
    FieldIntSub { local: usize, field_index: usize, x: usize },
    FieldIntMul { local: usize, field_index: usize, x: usize },
    FieldFloatAdd { local: usize, field_index: usize, x: usize },
    FieldFloatSub { local: usize, field_index: usize, x: usize },
    FieldFloatMul { local: usize, field_index: usize, x: usize },
    /// fused `GetLocal(s); GetLocal(v); FieldSet`. `s.f = v`. easy.
    FieldSetLocalLocal { local: usize, field_index: usize, v: usize },
    /// fused `GetLocal(s); Const(c); FieldSet`. `s.f = <const>` (`c` indexes the const pool).
    FieldSetLocalConst { local: usize, field_index: usize, c: usize },
}

impl OpCode {
    /// the instruction index this op can jump to, if it's a jump-like op.
    /// the peephole pass uses this to find jump targets. an instruction that is a
    /// target must never get folded into the middle of a fused op. ever.
    /// it also rewrites targets after folding. busy little pass.
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
/// shown in stack traces. the VM ignores it on the happy path. happy path = good.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loc {
    pub file: Option<std::rc::Rc<str>>,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub name: String,
    pub code: Vec<OpCode>,
    /// one entry per instruction in `code`, same order, don't shuffle
    pub locs: Vec<Loc>,
    /// location given to every instruction emitted from now on. sticky.
    pub cur_loc: Loc,
    pub constants: Vec<Value>,
    /// number of local slots this chunk needs (params + lets),
    /// allocated up front per call frame. no surprises.
    pub num_locals: usize,
    pub arity: usize,
    /// param and return kinds, when all of them are `int`, `float` or `bool`
    /// (set by the compiler). only these can run as native (spg-jit). the rest suffer in the VM.
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

    /// appends an instruction, returns its index (the compiler uses this
    /// to patch jump targets later. emit a placeholder Jump,
    /// remember the index, backpatch when the real target exists. fun.
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

    /// index the next emitted instruction will get, used to compute
    /// jump targets (e.g. "jump back to the top of this loop"). loops are cursed.
    pub fn next_index(&self) -> usize {
        self.code.len()
    }
}

/// A whole compiled program: one chunk per function, plus the
/// top-level chunk (`main`). function calls
/// point at other chunks by index into `functions`. spaghetti, as promised.
#[derive(Debug, Clone)]
pub struct Program {
    pub main: Chunk,
    pub functions: Vec<Chunk>,
    /// function name -> index into `functions`, so the VM/compiler can
    /// resolve a call by name without a linear scan. we're not animals.
    pub function_names: Vec<String>,
    /// struct definitions, indexed by `OpCode::MakeStruct`'s `def_index`
    /// (shared `Rc`, so every `Value::Struct` from the same type points
    /// at the same `StructDef` instead of cloning its
    /// name/field-name strings per instance. memory is not free.
    pub struct_defs: Vec<Rc<crate::value::StructDef>>,
    /// enum definitions, indexed by `Value::Enum`'s first field (only for
    /// printing a variant's name. comparing enums never looks. good.
    pub enum_defs: Vec<Rc<crate::value::EnumDef>>,
    /// how many global slots the VM must allocate before `main` runs.
    pub num_globals: usize,
}

/// a value kind the native backend understands. it's a small world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    Int,
    Float,
    Bool,
    /// a reference-counted string. native owns one ref per value it holds.
    String,
    /// an array whose elements are all this kind (arrays are refcounted too, sigh)
    Array(ElemKind),
    /// a struct of the type `Program.struct_defs[index]`
    Struct(usize),
}

/// element kinds an array can have in native (no nested arrays, we're not insane)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemKind {
    Int,
    Float,
    Bool,
    String,
}

impl ElemKind {
    /// the kind of one element, as a value on the native stack. simple.
    pub fn value_kind(self) -> NativeKind {
        match self {
            ElemKind::Int => NativeKind::Int,
            ElemKind::Float => NativeKind::Float,
            ElemKind::Bool => NativeKind::Bool,
            ElemKind::String => NativeKind::String,
        }
    }
}
