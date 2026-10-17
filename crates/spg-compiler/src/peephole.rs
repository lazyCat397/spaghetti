//! superinstruction pass: folds common instruction sequences of a compiled
//! `Chunk` into single fused opcodes. same behavior, fewer dispatches for
//! the VM. the dispatch (not the work inside an op) is what eats the time in an
//! interpreter loop, so cutting a loop body from ~24 ops to ~9 is worth
//! ~3x. kept separate from `compiler.rs` so normal emission stays
//! simple. this file is the only place to touch when tuning speed. the goblin's den.
//!
//! what gets fused (see `match_at` for the exact patterns, in priority order):
//! - `SetLocal; Pop` (every assignment) -> `StoreLocal`, same for globals
//! - local-with-constant arithmetic/compares (`i + 1`, `x % 3`, `n < 10`)
//!   and the in-place increment `i += k` (`IncLocal`). 
//! - every int compare followed by `JumpIfFalse` (stack / two locals /
//!   local-vs-const / stack-vs-const forms) -> one `JumpUnless*` op
//! - `GetLocal; GetLocal; <op>` for arithmetic, compares and 2-arg calls,
//!   plus the call shapes `f(a)` and `f(n - 1)`. fib lives here too.
//! - arithmetic whose result goes straight into a local (`x = a op b`)
//! - array/struct reads straight from a local (`a[i]`, `s.f`) and the
//!   `s.f OP= v` prefix
//! - loop inversion: the back-edge `Jump(head)` becomes the NEGATED head
//!   compare aimed at the loop body. one dispatch saved per iteration. free lunch.
//!
//! two invariants keep this from blowing up:
//! 1. an instruction that is the TARGET of a jump may only be the FIRST
//!    instruction of a fused op, never swallowed into the middle of one
//!    (`(if (c) { a } else { b }) < d` has a jump landing on the second
//!    `GetLocal` of a fusable triple. the pre-fix pass miscompiled it. oops.
//! 2. every jump target is remapped through the old->new index table.
//!
//! fusions chain, so passes repeat until one changes nothing. every
//! fusion was measured alone, and the ones that didn't help got
//! dropped (`GetLocal; JumpIfFalse`, for one, was slower. rip.).
//! `spg --no-opt` turns the whole pass off, and it's the reference the fused
//! bytecode must match. the truth source.

use spg_typecheck::BuiltinId;
use spg_vm::chunk::{Chunk, OpCode};
use spg_vm::value::Value;

/// maps the opcode that follows `GetLocal(a); GetLocal(b);` to its
/// fused form, if one exists. `None` means we don't fuse this shape, so
/// the loop below just copies the instructions as-is. boring, honest.
fn fused_replacement(op: OpCode, a: usize, b: usize) -> Option<OpCode> {
    match op {
        OpCode::AddInt => Some(OpCode::AddLocals(a, b)),
        OpCode::SubInt => Some(OpCode::SubLocals(a, b)),
        OpCode::MulInt => Some(OpCode::MulLocals(a, b)),
        OpCode::DivInt => Some(OpCode::DivLocals(a, b)),
        OpCode::ModInt => Some(OpCode::ModLocals(a, b)),
        OpCode::LtInt => Some(OpCode::LtLocals(a, b)),
        OpCode::GtInt => Some(OpCode::GtLocals(a, b)),
        OpCode::LtEqInt => Some(OpCode::LtEqLocals(a, b)),
        OpCode::GtEqInt => Some(OpCode::GtEqLocals(a, b)),
        OpCode::EqInt => Some(OpCode::EqIntLocals(a, b)),
        OpCode::NotEqInt => Some(OpCode::NotEqIntLocals(a, b)),

        // float/string/bool, mirroring the int set exactly (Add + the
        // 6 comparisons where the type supports them. copy paste with feelings.
        OpCode::AddFloat => Some(OpCode::AddFloatLocals(a, b)),
        OpCode::SubFloat => Some(OpCode::SubFloatLocals(a, b)),
        OpCode::MulFloat => Some(OpCode::MulFloatLocals(a, b)),
        OpCode::DivFloat => Some(OpCode::DivFloatLocals(a, b)),
        OpCode::ModFloat => Some(OpCode::ModFloatLocals(a, b)),
        OpCode::LtFloat => Some(OpCode::LtFloatLocals(a, b)),
        OpCode::GtFloat => Some(OpCode::GtFloatLocals(a, b)),
        OpCode::LtEqFloat => Some(OpCode::LtEqFloatLocals(a, b)),
        OpCode::GtEqFloat => Some(OpCode::GtEqFloatLocals(a, b)),
        OpCode::EqFloat => Some(OpCode::EqFloatLocals(a, b)),
        OpCode::NotEqFloat => Some(OpCode::NotEqFloatLocals(a, b)),
        OpCode::ConcatString => Some(OpCode::ConcatStringLocals(a, b)),
        OpCode::EqString => Some(OpCode::EqStringLocals(a, b)),
        OpCode::NotEqString => Some(OpCode::NotEqStringLocals(a, b)),
        OpCode::EqBool => Some(OpCode::EqBoolLocals(a, b)),
        OpCode::NotEqBool => Some(OpCode::NotEqBoolLocals(a, b)),

        // a 2-arg call where both args are bare locals. fancy.
        OpCode::Call { func_index, argc: 2 } => Some(OpCode::CallLocals2 { func_index, a, b }),
        _ => None,
    }
}


/// `GetLocal(a); Const(k); <int compare>` -> compare-local-with-constant
fn local_const_cmp(op: OpCode, a: usize, k: i64) -> Option<OpCode> {
    match op {
        OpCode::LtInt => Some(OpCode::LtLocalConst(a, k)),
        OpCode::GtInt => Some(OpCode::GtLocalConst(a, k)),
        OpCode::LtEqInt => Some(OpCode::LtEqLocalConst(a, k)),
        OpCode::GtEqInt => Some(OpCode::GtEqLocalConst(a, k)),
        OpCode::EqInt => Some(OpCode::EqLocalConst(a, k)),
        OpCode::NotEqInt => Some(OpCode::NotEqLocalConst(a, k)),
        _ => None,
    }
}

/// `<int compare in any of its three forms>; JumpIfFalse(t)` -> one fused jump
fn jump_unless(op: OpCode, t: usize) -> Option<OpCode> {
    match op {
        OpCode::LtInt => Some(OpCode::JumpUnlessLt(t)),
        OpCode::LtLocals(a, b) => Some(OpCode::JumpUnlessLtLocals(a, b, t)),
        OpCode::LtLocalConst(a, k) => Some(OpCode::JumpUnlessLtLocalConst(a, k, t)),
        OpCode::GtInt => Some(OpCode::JumpUnlessGt(t)),
        OpCode::GtLocals(a, b) => Some(OpCode::JumpUnlessGtLocals(a, b, t)),
        OpCode::GtLocalConst(a, k) => Some(OpCode::JumpUnlessGtLocalConst(a, k, t)),
        OpCode::LtEqInt => Some(OpCode::JumpUnlessLtEq(t)),
        OpCode::LtEqLocals(a, b) => Some(OpCode::JumpUnlessLtEqLocals(a, b, t)),
        OpCode::LtEqLocalConst(a, k) => Some(OpCode::JumpUnlessLtEqLocalConst(a, k, t)),
        OpCode::GtEqInt => Some(OpCode::JumpUnlessGtEq(t)),
        OpCode::GtEqLocals(a, b) => Some(OpCode::JumpUnlessGtEqLocals(a, b, t)),
        OpCode::GtEqLocalConst(a, k) => Some(OpCode::JumpUnlessGtEqLocalConst(a, k, t)),
        OpCode::EqInt => Some(OpCode::JumpUnlessEq(t)),
        OpCode::EqIntLocals(a, b) => Some(OpCode::JumpUnlessEqLocals(a, b, t)),
        OpCode::EqLocalConst(a, k) => Some(OpCode::JumpUnlessEqLocalConst(a, k, t)),
        OpCode::NotEqInt => Some(OpCode::JumpUnlessNotEq(t)),
        OpCode::NotEqIntLocals(a, b) => Some(OpCode::JumpUnlessNotEqLocals(a, b, t)),
        OpCode::NotEqLocalConst(a, k) => Some(OpCode::JumpUnlessNotEqLocalConst(a, k, t)),
        _ => None,
    }
}


/// `<Add|Sub|Mul>(Int|Float); StoreLocal(a)` -> op that stores into the slot
fn op_store(op: OpCode, a: usize) -> Option<OpCode> {
    match op {
        OpCode::AddInt => Some(OpCode::AddIntStore(a)),
        OpCode::SubInt => Some(OpCode::SubIntStore(a)),
        OpCode::MulInt => Some(OpCode::MulIntStore(a)),
        OpCode::AddFloat => Some(OpCode::AddFloatStore(a)),
        OpCode::SubFloat => Some(OpCode::SubFloatStore(a)),
        OpCode::MulFloat => Some(OpCode::MulFloatStore(a)),
        _ => None,
    }
}

/// `Const(k); <int compare>; JumpIfFalse(t)`
fn const_cmp_jump(op: OpCode, k: i64, t: usize) -> Option<OpCode> {
    match op {
        OpCode::LtInt => Some(OpCode::JumpUnlessLtConst(k, t)),
        OpCode::GtInt => Some(OpCode::JumpUnlessGtConst(k, t)),
        OpCode::LtEqInt => Some(OpCode::JumpUnlessLtEqConst(k, t)),
        OpCode::GtEqInt => Some(OpCode::JumpUnlessGtEqConst(k, t)),
        OpCode::EqInt => Some(OpCode::JumpUnlessEqConst(k, t)),
        OpCode::NotEqInt => Some(OpCode::JumpUnlessNotEqConst(k, t)),
        _ => None,
    }
}


/// loop inversion. a `while`/`for` back-edge is `Jump(head)` where `head` is
/// a single fused compare `JumpUnless<rel>(.., exit)` and `exit` is the
/// instruction right after the back-edge. replace the back-edge with the
/// NEGATED compare aimed at the first body instruction. it jumps into the
/// body while the condition holds, and falls through to `exit`
/// otherwise. same behaviour, one dispatch less per iteration. (the classic trick.)
fn invert_loops(chunk: &mut Chunk) {
    for i in 0..chunk.code.len() {
        let OpCode::Jump(head) = chunk.code[i] else { continue };
        if head >= i {
            continue;
        }
        let (exit, body) = (i + 1, head + 1);
        let inverted = match chunk.code[head] {
            OpCode::JumpUnlessLtLocals(a, b, e) if e == exit => OpCode::JumpUnlessGtEqLocals(a, b, body),
            OpCode::JumpUnlessLtLocalConst(a, k, e) if e == exit => OpCode::JumpUnlessGtEqLocalConst(a, k, body),
            OpCode::JumpUnlessGtLocals(a, b, e) if e == exit => OpCode::JumpUnlessLtEqLocals(a, b, body),
            OpCode::JumpUnlessGtLocalConst(a, k, e) if e == exit => OpCode::JumpUnlessLtEqLocalConst(a, k, body),
            OpCode::JumpUnlessLtEqLocals(a, b, e) if e == exit => OpCode::JumpUnlessGtLocals(a, b, body),
            OpCode::JumpUnlessLtEqLocalConst(a, k, e) if e == exit => OpCode::JumpUnlessGtLocalConst(a, k, body),
            OpCode::JumpUnlessGtEqLocals(a, b, e) if e == exit => OpCode::JumpUnlessLtLocals(a, b, body),
            OpCode::JumpUnlessGtEqLocalConst(a, k, e) if e == exit => OpCode::JumpUnlessLtLocalConst(a, k, body),
            OpCode::JumpUnlessEqLocals(a, b, e) if e == exit => OpCode::JumpUnlessNotEqLocals(a, b, body),
            OpCode::JumpUnlessEqLocalConst(a, k, e) if e == exit => OpCode::JumpUnlessNotEqLocalConst(a, k, body),
            OpCode::JumpUnlessNotEqLocals(a, b, e) if e == exit => OpCode::JumpUnlessEqLocals(a, b, body),
            OpCode::JumpUnlessNotEqLocalConst(a, k, e) if e == exit => OpCode::JumpUnlessEqLocalConst(a, k, body),
            _ => continue,
        };
        chunk.code[i] = inverted;
    }
}

/// tries every fusion pattern starting at `code[i]` and returns the fused op
/// plus how many instructions it replaces. priority is top to bottom (longest
/// patterns first). the order matters. validated by differential
/// fuzzing against the unfused bytecode. `limit` is the biggest span allowed:
/// a pattern may not swallow an instruction that's a jump target
/// (only its FIRST instruction may be one), because control coming from the
/// jump would land on the fused op and skip the folded instructions. chaos averted.
/// patterns are written over the output of earlier passes too (e.g.
/// `AddLocalConst; StoreLocal` only exists once both halves got fused).
fn match_at(code: &[OpCode], consts: &[Value], i: usize, limit: usize, late: bool) -> Option<(OpCode, usize)> {
    let at = |k: usize| code.get(i + k).copied();
    let int_const = |op: OpCode| match op {
        OpCode::Const(ci) => match consts.get(ci) {
            Some(Value::Int(k)) => Some(*k),
            _ => None,
        },
        _ => None,
    };

    // `@array.len(a)` / `len(a)` is the dedicated opcode (same behavior, no builtin call)
    if let OpCode::CallBuiltin { id: BuiltinId::ArrayLenBuiltin, argc: 1 } = code[i] {
        return Some((OpCode::ArrayLen, 1));
    }

    // ---- 4 instructions (the big ones) ----
    if limit >= 4 {
        // `arr[i] = v` / `arr[i] = <constant>`: the whole statement
        if let (OpCode::GetLocal(arr), Some(OpCode::GetLocal(idx)), Some(val), Some(OpCode::IndexSet)) = (code[i], at(1), at(2), at(3)) {
            match val {
                OpCode::GetLocal(v) => return Some((OpCode::IndexSetLocalsLocal { arr, idx, v }, 4)),
                OpCode::Const(c) => return Some((OpCode::IndexSetLocalsConst { arr, idx, c }, 4)),
                _ => {}
            }
        }
        // `s.f OP= x` with a local x: the whole statement
        if let (OpCode::FieldGetDup { local, field_index }, Some(OpCode::GetLocal(x)), Some(op), Some(OpCode::FieldSet { field_index: f2 })) =
            (code[i], at(1), at(2), at(3))
        {
            if f2 == field_index {
                let f = match op {
                    OpCode::AddInt => Some(OpCode::FieldIntAdd { local, field_index, x }),
                    OpCode::SubInt => Some(OpCode::FieldIntSub { local, field_index, x }),
                    OpCode::MulInt => Some(OpCode::FieldIntMul { local, field_index, x }),
                    OpCode::AddFloat => Some(OpCode::FieldFloatAdd { local, field_index, x }),
                    OpCode::SubFloat => Some(OpCode::FieldFloatSub { local, field_index, x }),
                    OpCode::MulFloat => Some(OpCode::FieldFloatMul { local, field_index, x }),
                    _ => None,
                };
                if let Some(f) = f {
                    return Some((f, 4));
                }
            }
        }
        // `arr[i] OP= x` with a local x: the whole statement
        if let (OpCode::IndexGetDupLocals(arr, idx), Some(OpCode::GetLocal(x)), Some(op), Some(OpCode::IndexSet)) =
            (code[i], at(1), at(2), at(3))
        {
            let f = match op {
                OpCode::AddInt => Some(OpCode::IndexAddLocalLocals { arr, idx, x }),
                OpCode::SubInt => Some(OpCode::IndexSubLocalLocals { arr, idx, x }),
                OpCode::MulInt => Some(OpCode::IndexMulLocalLocals { arr, idx, x }),
                OpCode::AddFloat => Some(OpCode::IndexAddFloatLocalLocals { arr, idx, x }),
                OpCode::SubFloat => Some(OpCode::IndexSubFloatLocalLocals { arr, idx, x }),
                OpCode::MulFloat => Some(OpCode::IndexMulFloatLocalLocals { arr, idx, x }),
                _ => None,
            };
            if let Some(f) = f {
                return Some((f, 4));
            }
        }
        // `arr[i] OP= v` starts with this
        if let (OpCode::GetLocal(arr), Some(OpCode::GetLocal(idx)), Some(OpCode::Dup2), Some(OpCode::IndexGet)) =
            (code[i], at(1), at(2), at(3))
        {
            return Some((OpCode::IndexGetDupLocals(arr, idx), 4));
        }
        // `fib(n - 1)`. the poster child.
        if let (OpCode::GetLocal(a), Some(c), Some(OpCode::SubInt), Some(OpCode::Call { func_index, argc: 1 })) =
            (code[i], at(1), at(2), at(3))
        {
            if let Some(k) = int_const(c) {
                return Some((OpCode::CallLocalMinusConst1 { func_index, a, k }, 4));
            }
        }
    }

    // ---- 2 instructions that build on fused forms (fused on fused, yo) ----
    if limit >= 2 {
        // `x += k` in place
        if let (OpCode::AddLocalConst(a, k), Some(OpCode::StoreLocal(b))) = (code[i], at(1)) {
            if a == b {
                return Some((OpCode::IncLocal(a, k), 2));
            }
        }
        // compare + JumpIfFalse (stack, two-locals and local-vs-constant forms)
        if let Some(OpCode::JumpIfFalse(t)) = at(1) {
            if let Some(f) = jump_unless(code[i], t) {
                return Some((f, 2));
            }
        }
    }

    // ---- 3 instructions ----
    if limit >= 3 {
        if let (OpCode::GetLocal(a), Some(c), Some(third)) = (code[i], at(1), at(2)) {
            if let Some(k) = int_const(c) {
                let f = match third {
                    OpCode::AddInt => Some(OpCode::AddLocalConst(a, k)),
                    OpCode::SubInt => Some(OpCode::AddLocalConst(a, k.wrapping_neg())),
                    OpCode::MulInt => Some(OpCode::MulLocalConst(a, k)),
                    // a zero divisor must keep failing at runtime. not fused. no shortcuts.
                    OpCode::DivInt if k != 0 => Some(OpCode::DivLocalConst(a, k)),
                    OpCode::ModInt if k != 0 => Some(OpCode::ModLocalConst(a, k)),
                    other => local_const_cmp(other, a, k),
                };
                if let Some(f) = f {
                    return Some((f, 3));
                }
            }
        }
        if let (OpCode::GetLocal(arr), Some(OpCode::GetLocal(idx)), Some(OpCode::IndexGet)) = (code[i], at(1), at(2)) {
            return Some((OpCode::IndexGetLocals(arr, idx), 3));
        }
        // `arr[i] += k`: the whole statement
        if let (OpCode::IndexGetDupLocals(arr, idx), Some(OpCode::AddConst(k)), Some(OpCode::IndexSet)) = (code[i], at(1), at(2)) {
            return Some((OpCode::IndexAddConstLocals { arr, idx, k }, 3));
        }
        // `GetLocal(a); GetLocal(b); <op>` for every arithmetic/compare/call op
        if let (OpCode::GetLocal(a), Some(OpCode::GetLocal(b)), Some(third)) = (code[i], at(1), at(2)) {
            if let Some(f) = fused_replacement(third, a, b) {
                return Some((f, 3));
            }
        }
    }

    // ---- 2 instructions (the small fry) ----
    if limit >= 2 {
        if let (OpCode::GetLocal(a), Some(OpCode::Call { func_index, argc: 1 })) = (code[i], at(1)) {
            return Some((OpCode::CallLocal1 { func_index, a }, 2));
        }
    }

    if limit >= 3 {
        // `x % 2 == 0`: constant compare + jump
        if let (c, Some(cmp), Some(OpCode::JumpIfFalse(t))) = (code[i], at(1), at(2)) {
            if let Some(k) = int_const(c) {
                if let Some(f) = const_cmp_jump(cmp, k, t) {
                    return Some((f, 3));
                }
            }
        }
        // `m[i][j] = v`: the row is already on the stack
        if let (OpCode::GetLocal(idx), Some(OpCode::GetLocal(v)), Some(OpCode::IndexSet)) = (code[i], at(1), at(2)) {
            return Some((OpCode::IndexSetLocalLocal { idx, v }, 3));
        }
        // `s.f += k`: the whole statement
        if let (OpCode::FieldGetDup { local, field_index }, Some(OpCode::AddConst(k)), Some(OpCode::FieldSet { field_index: f2 })) =
            (code[i], at(1), at(2))
        {
            if f2 == field_index {
                return Some((OpCode::FieldAddConst { local, field_index, k }, 3));
            }
        }
        // `s.f = v` / `s.f = <constant>`
        if let (OpCode::GetLocal(local), Some(val), Some(OpCode::FieldSet { field_index })) = (code[i], at(1), at(2)) {
            match val {
                OpCode::GetLocal(v) => return Some((OpCode::FieldSetLocalLocal { local, field_index, v }, 3)),
                OpCode::Const(c) => return Some((OpCode::FieldSetLocalConst { local, field_index, c }, 3)),
                _ => {}
            }
        }
        // `s.f OP= v` starts with this
        if let (OpCode::GetLocal(l), Some(OpCode::Dup1), Some(OpCode::FieldGet { field_index })) = (code[i], at(1), at(2)) {
            return Some((OpCode::FieldGetDup { local: l, field_index }, 3));
        }
    }

    if limit >= 2 {
        if let Some(OpCode::StoreLocal(d)) = at(1) {
            // arithmetic whose result goes straight into a local
            if let Some(f) = op_store(code[i], d) {
                return Some((f, 2));
            }
            let f = match code[i] {
                OpCode::AddLocals(a, b) => Some(OpCode::AddLocalsStore(d, a, b)),
                OpCode::SubLocals(a, b) => Some(OpCode::SubLocalsStore(d, a, b)),
                OpCode::MulLocals(a, b) => Some(OpCode::MulLocalsStore(d, a, b)),
                _ => None,
            };
            if let Some(f) = f {
                return Some((f, 2));
            }
        }

        match (code[i], at(1)) {
            // assignment statements: set + discard. the usual dance.
            (OpCode::SetLocal(a), Some(OpCode::Pop)) => return Some((OpCode::StoreLocal(a), 2)),
            (OpCode::SetGlobal(a), Some(OpCode::Pop)) => return Some((OpCode::StoreGlobal(a), 2)),
            (OpCode::GetLocal(a), Some(OpCode::Return)) => return Some((OpCode::ReturnLocal(a), 2)),
            (OpCode::GetLocal(i2), Some(OpCode::IndexGet)) => return Some((OpCode::IndexGetLocal(i2), 2)),
            (OpCode::GetLocal(l), Some(OpCode::FieldGet { field_index })) => {
                return Some((OpCode::FieldGetLocal { local: l, field_index }, 2))
            }
            (c, Some(third)) => {
                if let Some(k) = int_const(c) {
                    let f = match third {
                        OpCode::AddInt => Some(OpCode::AddConst(k)),
                        OpCode::SubInt => Some(OpCode::AddConst(k.wrapping_neg())),
                        OpCode::MulInt => Some(OpCode::MulConst(k)),
                        OpCode::DivInt if k != 0 => Some(OpCode::DivConst(k)),
                        OpCode::ModInt if k != 0 => Some(OpCode::ModConst(k)),
                        _ => None,
                    };
                    if let Some(f) = f {
                        return Some((f, 2));
                    }
                }
            }
            _ => {}
        }
    }

    // a bare `GetLocal; GetLocal` pair only runs last, so it can't steal an
    // operand from one of the bigger patterns above. manners.
    if late && limit >= 2 {
        if let (OpCode::GetLocal(a), Some(OpCode::GetLocal(b))) = (code[i], at(1)) {
            return Some((OpCode::GetLocal2(a, b), 2));
        }
    }

    None
}

pub fn fuse_superinstructions(chunk: &mut Chunk) {
    // fused ops can be part of a bigger pattern too, so keep going
    // until a whole pass finds nothing left to fold. wait for it.
    while fuse_pass(chunk, false) {}
    // low-value patterns that could steal operands from bigger ones (a bare
    // `GetLocal; GetLocal` pair) only run once everything else has settled
    fuse_pass(chunk, true);
    {
        invert_loops(chunk);
    }
}

fn fuse_pass(chunk: &mut Chunk, late: bool) -> bool {
    let old = std::mem::take(&mut chunk.code);
    let old_locs = std::mem::take(&mut chunk.locs);
    let n = old.len();

    let mut is_target = vec![false; n + 1];
    for op in &old {
        if let Some(t) = op.jump_target() {
            is_target[t] = true;
        }
    }

    let mut new_code = Vec::with_capacity(n);
    // a fused op reports the location of its first instruction
    let mut new_locs = Vec::with_capacity(n);
    // old index -> index in `new_code`. jumps get rewritten through this
    let mut map = vec![0usize; n + 1];
    let mut changed = false;

    let mut i = 0;
    while i < n {
        // span of instructions starting at `i` that has no jump
        // target, except maybe `i` itself
        let mut limit = 1;
        while i + limit < n && !is_target[i + limit] {
            limit += 1;
        }
        let (op, len) = match match_at(&old, &chunk.constants, i, limit, late) {
            Some((op, len)) => {
                changed = true;
                (op, len)
            }
            None => (old[i], 1),
        };
        let ni = new_code.len();
        for k in 0..len {
            map[i + k] = ni;
        }
        new_code.push(op);
        new_locs.push(old_locs[i].clone());
        i += len;
    }
    // a jump may target one-past-the-end (exit point after a loop or if). the exit door.
    map[n] = new_code.len();

    for op in &mut new_code {
        if let Some(t) = op.jump_target_mut() {
            *t = map[*t];
        }
    }
    chunk.code = new_code;
    chunk.locs = new_locs;
    changed
}
