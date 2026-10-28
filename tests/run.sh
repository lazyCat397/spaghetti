#!/bin/bash
# tick: rewrites the same terminal line after each test (update-spg.bat waits for this line).
# say: clears that line before any normal message (FAIL, skip, summary) so nothing sticks to it.
tick() { printf '\r[progress] %d passed out of %d, %d failed' "$pass" "$TOTAL" "$fail"; }
say()  { printf '\r\033[K%s\n' "$*"; }
# Multi-file regression. Checks the EXACT OUTPUT (not just the exit code):
# a renaming bug usually gives a wrong result without crashing.
# Usage (from the repository root): bash tests/run.sh
cd "$(dirname "$0")"
SPG=${SPG:-../target/release/spg}
# the ui library needs a display: the suite runs it with the null backend (no window opens)
export SPG_UI_BACKEND=null
# [progress] needs the total (Y), counted WITHOUT running anything: the direct calls
# (ok/err/exact1/lines/diag/exit124/check), the same/ loop (one per file) and the standalone checks.
count_tests() {
    local n loopn avul f
    n=$(grep -cE '^(ok|err|exact1|lines|diag|exit124|check) ' run.sh)
    loopn=0
    for f in ok/*.spg lang/*.spg ../examples/*.spg; do
        case "$f" in *time_test*|*sleep*|*ps_missing*) continue;; esac
        loopn=$((loopn+1))
    done
    avul=4                                   # build, fmt, ps_missing, ps_safe
    if [[ "$OSTYPE" != msys* && "$OSTYPE" != cygwin* ]] && command -v python3 >/dev/null 2>&1; then
        avul=$((avul+1))                     # timing/flush_latency
    fi
    echo $((n+loopn+avul))
}
TOTAL=$(count_tests)
# norm: makes a program's output independent of the platform's path style. Windows
# prints `err\x.spg` and may add a `\\?\C:\...` prefix; the expectations use `err/x.spg`.
# Only the directories before `err` or `ok` are touched, so `"a\nb"` printed by a program stays as it is.
norm() { sed -E 's#[^ ()]*(err|ok)[\\/]#\1/#g'; }
# the normalizer itself: Windows-style paths (with and without the \\?\ prefix) must match the expectations
if [ "$(printf '%s' 'at f (C:\a\b\err\x.spg:2) and \\?\C:\a\err\y.spg, keep a\nb' | norm)" != 'at f (err/x.spg:2) and err/y.spg, keep a\nb' ]; then
    say "FAIL norm: the path normalizer changed the wrong text"; exit 1
fi
spgrun() {   # stdin: a pipe from the caller is passed through; a terminal is cut off (a program that reads input would otherwise wait on the keyboard for the whole 30 s)
    if [ -t 0 ]; then timeout 30 "$SPG" "$@" </dev/null; else timeout 30 "$SPG" "$@"; fi
}   # a hung program (infinite loop) becomes a clean FAIL (exit 124), not a stuck suite
exec </dev/null   # no test may wait on the keyboard (tests that need input pipe it in)
pass=0; fail=0
tick

ok() {   # ok <file> <expected output, with \n>
    raw=$(spgrun "ok/$1" 2>&1); code=$?; got=$(printf '%s' "$raw" | norm)
    want=$(printf -- "$2")
    if [ "$code" = 0 ] && [ "$got" = "$want" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL ok/$1 (exit=$code)"; echo "  want: $(echo "$want" | tr '\n' '|')"; echo "  got:  $(echo "$got" | tr '\n' '|')"; fi
}
err() {  # err <file> <fragment> [<fragment>...]  — ALL must appear
    f="$1"; shift
    raw=$(spgrun "err/$f" 2>&1); code=$?; got=$(printf '%s' "$raw" | norm)
    miss=""
    for want in "$@"; do echo "$got" | grep -qF -- "$want" || miss="$want"; done
    if [ "$code" = 1 ] && [ -z "$miss" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL err/$f (exit=$code)"; echo "  faltou: $miss"; echo "  got: $got"; fi
}

ok basic.spg            '42\n6.28\n0\n7\n42'
ok newline_statements.spg '1\n5\n1'
ok diamond.spg          '120\n120'
ok same_name.spg        '10'
ok struct_forms.spg           '11\n21\n4\n6\n77\n40'
ok shadowing.spg         '6\n7\n500\n1000\n500'
ok builtin_homonimo.spg  '1001\n3'
ok subpastas.spg         'middle+leaf'
ok reexport_simples.spg     '2\n7'
ok reexport_identidade.spg  '3'
ok reexport_cadeia.spg      '2'
ok reexport_mixed_exports.spg       '0\n100\n5'
ok reexport_private_same_name.spg '999'
ok ns_modulo.spg         '5.0\nHI\ntrue'
ok ns_all.spg          '5\nSPG\ntrue\n2\ntrue\n2\n4\na+b+c\n9\n2.5\n3\n42true\n5.0\n5\n124\nfalse\ntrue\ntrue\ntrue\n20\nT\ntrue'
ok ns_free_names.spg   '6\n6\n4.0\nT'
ok ns_into.spg           '42\n5\n-5\n42\n7\n3.0\n3.5\n42true1.5\n3.0'
ok ns_pi_e.spg           '3.1416\n3.0\n3.1\n3.14\n3.14\n3.141592653589793\n3.141592653589793\n2.718\n2.72\n3.0\n3.14159'
ok str_chars.spg         '5\n5\n3\nï\naç\nNAÏVE\n5\ná|é|í\nééé\n3\ntrue\ntrue'

# buffered stdout (print goes through a 64 KB buffer when stdout is not a terminal).
# These prove ORDER and COMPLETENESS, which is what buffering could break.
exact1() {  # exact1 <file> <expected output> — exit code 1 and EXACT output (stdout+stderr), in order
    raw=$(spgrun "err/$1" 2>&1); code=$?; got=$(printf '%s' "$raw" | norm)
    want=$(printf -- "$2")
    if [ "$code" = 1 ] && [ "$got" = "$want" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL err/$1 (exit=$code)"; echo "  want: $(echo "$want" | tr '\n' '|')"; echo "  got:  $(echo "$got" | tr '\n' '|')"; fi
}
lines() {  # lines <file> <number of lines> <last line> — nothing may be lost beyond the buffer
    raw=$(spgrun "ok/$1" 2>&1); code=$?; got=$(printf '%s' "$raw" | norm)
    n=$(echo "$got" | wc -l); last=$(echo "$got" | tail -1)
    if [ "$code" = 0 ] && [ "$n" = "$2" ] && [ "$last" = "$3" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL ok/$1 (exit=$code, lines=$n, last=$last)"; fi
}
ok enum_scope.spg  '7\nGreen\n1\n2\nRed'
ok float_display.spg           '3.0\n2.0\n4.0\n-0.0\n0.5'
ok interpolation.spg         'hi Ana, I am 30 years old\nsum 31 and 3 items\nnested: xAnay / 1\nraw ${name} and $5 and 6\n1.5|true|[1, 2, 3]\n3'
ok dir_wrap_default.spg         'wrap\n-9223372036854775808\n0'
ok json_set.spg    '{\n    "version": "0.15.0",\n    "items": [\n        1,\n        2,\n        3\n    ],\n    "author": "ana"\n}\n{"a":2}\n{"a":1,"tags":["x","y"]}\n3'
ok map_keysof.spg '[Eggs, Résumé]\n[Butter] []\n[1, 3] 2\n[Milk]\n[]'
ok multiline_expr.spg      '3\n10'
ok ns_campos.spg         '7\nx'
ok ns_str_pop.spg  '[007] [   ab] [abxyx] [ab   ]\nlong x ··é\n3 -1 3 1\n[hi  ] [  hi] [hi]\n3 2 [1] 1\n[[], [1]] [[1], []]'
ok print_order.spg       'a\nE1\nb\nE2\nc'
lines print_big.spg 30000 29999
exact1 print_before_error.spg 'a\nE1\nb\nruntime error: division by zero\n  at <main> (err/print_before_error.spg:5)'

# fused array-index ops (hand-verified values; the OOB errors must keep the exact message)
ok index_fused.spg       '[3, 2, 15, 7]\n[1.75, 3.25]\n[false, true, true]\n[[1, 12], [3, 3]]\n5\n9'
err default_before_required.spg 'parameters after one with a default need one too'
err default_not_literal.spg 'a literal default value'
err index_fused_oob_addconst.spg 'index out of bounds: 9 (array has 3 element(s))'
err index_fused_oob_addlocal.spg 'index out of bounds: 9 (array has 3 element(s))'
err index_fused_oob_setlocal.spg 'index out of bounds: -1 (array has 3 element(s))'
err index_fused_oob_setconst.spg 'index out of bounds: 3 (array has 3 element(s))'
err index_fused_oob_set2d.spg    'index out of bounds: 5 (array has 2 element(s))'
err index_fused_oob_head.spg      'index out of bounds: 7 (array has 3 element(s))'

# builtins on the operand stack (call_builtin_on_stack) and the allocation-free charAt/substring (ASCII and UTF-8)
ok builtins_fast.spg     '5\n5\n5\n5\ntrue\nfalse\ntrue\nfalse\n[3, 1, 4, 1, 5, 9]\n9\n5\n[3, 1, 4, 1]\n7\n3.5\n-7\n4\n-3.5\n2.5\n-4\n-3\n3\n-4\n4.0\n-7.0\n-3\n4'
ok str_chars_utf8.spg    'a\nf\nï\nr\nç\na\né\nc\ncd\nïve \nnaïve wor\naé\n[]\n[]\nn\na\nï\nv\ne\n \nw\no\nr\nl\nd\n11'
err charat_oob.spg          'index out of bounds: 3 (array has 3 element(s))'
err charat_neg.spg          'index out of bounds: -1 (array has 3 element(s))'
err charat_oob_utf8.spg     'index out of bounds: 5 (array has 5 element(s))'
err substring_oob_utf8.spg  'index out of bounds: 9 (array has 5 element(s))'
err substring_range.spg     'index out of bounds: 1 (array has 3 element(s))'
err substring_neg.spg       'index out of bounds: 2 (array has 3 element(s))'

# stdout buffering with the timed flusher thread: nothing lost or reordered under load...
# fused struct-field ops (hand-verified; includes reference semantics: `let u = s; u.a += 1` changes s)
ok field_fused.spg       '57\n14\n0.875\n7\n99\n0.5\n8\n297\n100\n200\n9.5\n305'

lines print_stress.spg 300000 299999
# ...and nothing held back: the first print must reach a pipe while the program is still crunching
# (the program is killed as soon as it shows up, so this takes ~15 ms)
if [[ "$OSTYPE" == msys* || "$OSTYPE" == cygwin* ]]; then say "skip timing/flush_latency (Windows: select() does not work em pipes)"
elif command -v python3 >/dev/null 2>&1; then
    if python3 - "$SPG" timing/flush_latency.spg <<'PYEOF'
import subprocess, sys, time, select
st = time.perf_counter()
p = subprocess.Popen([sys.argv[1], sys.argv[2]], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
got = b""; still = False
while time.perf_counter() - st < 2.0:
    r, _, _ = select.select([p.stdout], [], [], 0.01)
    if r:
        got = p.stdout.read1(100); still = p.poll() is None; break
p.kill(); p.wait()
sys.exit(0 if got == b"one\n" and still and time.perf_counter() - st < 1.0 else 1)
PYEOF
    then pass=$((pass+1)); tick; else fail=$((fail+1)); tick; say "FAIL timing/flush_latency.spg: the 1st print did not reach the pipe while the program was computing"; fi
else say "skip timing/flush_latency (no python3)"; fi

# optimizer regressions (hand-verified expected values)
ok opt_jump_targets.spg  'true\nfalse\n13\n6\n8\n18\n1\n0\n1'
ok opt_loops.spg         '16\n0\n55\n0\n6\n1\n8\n0\n35\n12'
ok opt_ops.spg           '27\n13\n60\n5\n0\n60\n17\n37\n35\n15\n40\n-2\n-16\n-27\n-2\n-1\n-36\n-13\n-22\n-24\n-14\n-17\n11100\n1100101\n1101010\n6\n59\n4.25\n27\n50'

# '...' raw strings (no escapes, may hold "double quotes") and ranges: for i in a..b [step s] / @array.range
# (expected values verified by hand, line by line)
ok raw_strings.spg       'print("hi")\nhi\n15\na\\nb\n4\na\nb\n3\nnaïve\n\ntrue\n1\nabc\nmulti\nline'
ok range_for.spg         '1\n2\n3\n-\n0\n3\n6\n9\n- down\n5\n3\n1\n3\n2\n1\n- empty\n- non-literal step\n0\n3\n6\n7\n4\n1\n0\n3\n6\n9\n- continue / break\n1\n3\n5\n7\n9\n3\n- reassigning i does not move the counter\n10\n11\n12\n100\n101\n102\n- expressions as bounds, evaluated once\n0\n1\n2\n1\n5050\n10\n- loop variable shadows an outer one, bounds still see the outer\n0\n1\n2\n3\n- array.range\n[1, 2, 3, 4]\n[0, 3, 6, 9]\n[5, 3, 1]\n[]\n[]\n[]\n[-5, 0]\n[0]\n[9223372036854775800, 9223372036854775803, 9223372036854775806]\n0\n3\n6\n1000\n- extremes\n9223372036854775805\n9223372036854775806'
err range_step_zero.spg          'a range step cannot be zero' 'range_step_zero.spg at line 1'
err range_start_float.spg        'the start of a range must be int, found float'
err range_end_string.spg         'the end of a range must be int, found string'
err range_step_float.spg         'the step of a range must be int, found float'
exact1 range_step_zero_dynamic.spg 'before\nruntime error: range step cannot be zero\n  at f (err/range_step_zero_dynamic.spg:2)\n  at <main> (err/range_step_zero_dynamic.spg:7)'
err range_array_step_zero.spg    'runtime error: range step cannot be zero'
err range_array_too_large.spg    '@array.range would create 100000000 elements (the maximum is 20000000)' "use 'for i in a..b'"
err range_outside_for.spg        "'..' only works in a for loop header" "'@array.range(1, 4)'" '(line 1, column 10)'
err range_array_arity.spg        "'@array.range' expects 2 argument(s), got 1"
err type_arity.spg               "'type' expects 1 argument(s), got 0"
err union_arg_mismatch.spg       "'f' argument #1 expects string or float, got bool"
err union_eq_mismatch.spg        "operator Eq not supported for string or int and bool"
err union_arith.spg              "operator Add not supported for string or int and int"
err array_expected_let.spg      "this array's items are int, found string"
err array_expected_assign.spg   "this array's items are int, found bool"
err array_expected_arg.spg      "this array's items are int, found string"
err field_expected_assign.spg   "this array's items are int, found bool"
err map_expected_value.spg      "this map's values are int, found string"
err map_expected_key_union.spg  "map keys must be int, string, bool or an enum, found string or int"
err range_array_float.spg        "'@array.range' argument #1 expects int, got float"
err raw_unterminated.spg         'unterminated string, started at line 1, column 9'
err raw_glued.spg                'expected '"')'"', found Ident("hi")' 'line 1, column 15'
err raw_no_quote_escape.spg      'unterminated string, started at line 1, column 35'

# struct methods: `function` inside a struct, called `p.f()`; `me` is the receiver (values verified by hand)
ok struct_methods.spg       '7\n(6, 8)\n100\n2\n3\n5\n120\ntrue\nn=5\n6\n36\n11\n30\n9\n10\n101\n1\n6\nhi\n-1\n14'
ok struct_methods_multi.spg '25\n41\n41\nother:t\n42\n9'
err method_unknown.spg        "struct 'P' has no method 'nope'" 'line 12'
err method_field_called.spg   "'x' is a field of struct 'P', not a method"
err method_arity_few.spg      "'P.add' expects 1 argument(s), got 0"
err method_arity_many.spg     "'P.add' expects 1 argument(s), got 2"
err method_argtype.spg        "'P.add' argument #1 expects int, got string"
err method_on_int.spg         "type int has no method 'get'" "@math and @into functions that take it first"
err method_not_called.spg     "'get' is a method of struct 'P', not a field; call it with '.get()'"
err method_field_clash.spg    "struct 'Q' has both a field and a method named 'x'" '(line 3, column 5)'
err method_me_param.spg       "'me' is already declared in this scope"
err method_duplicate.spg      "'Q.f' is declared more than once" 'method_duplicate.spg:6' "'Q.f' is first declared here"
err method_bad_field.spg      "struct 'Q' has no field 'zzz'"
err method_fields_need_comma.spg "expected '}', found Ident" 'line 2, column 12'
err method_return_mismatch.spg 'function returns int but this return gives string'

# print / printError take any number of arguments (values joined by one space; no arguments = an empty line)
ok print_variadic.spg    '\na\n1 2\nx 1 2.5 true raw "q"\n[1, 2] arr\n3 3\n1 2 3 4 5 6 7 8 9 10\n1 2 3\n3\n \n  \nonly \n\ne 1 false\nafter\n\nlast'

err cycle_a.spg          'circular import'
err ciclo3_a.spg         'ciclo3_b.spg -> '
err auto_import.spg      'circular import'
err not_exported.spg    'is not exported'
err inexistente.spg      "has no declaration named 'fantasma'"
err no_extension.spg     "must end in '.spg'"
err missing_file.spg 'could not read'
err import_collision.spg  "'helper' is imported more than once" 'import_collision.spg:2' "'helper' is first imported here" 'import_collision.spg:1'
err local_conflict.spg    "'helper' conflicts with a local declaration" 'local_conflict.spg:1' "'helper' is also declared here" 'local_conflict.spg:2'
err local_duplicate.spg "'a' is declared more than once" 'local_duplicate.spg:3' "'a' is first declared here" 'local_duplicate.spg:1'
err import_clash_list.spg "'two' is imported more than once" "'two' is listed twice in this import"
err reexport_import_not_reexported.spg "only imported by module" 'export { inc } from "./reexport_origin.spg"'
err reexport_inexistente.spg    "has no declaration named 'fantasma'"
err reexport_private.spg        "'privada' exists in module" "is not exported"
err reexport_no_extension.spg   "must end in '.spg'"
err reexport_cycle.spg          'circular import' 'reexport_cycle_a.spg -> ' 'reexport_cycle_b.spg'
err reexport_vs_local.spg       "'inc' is re-exported here, but this file also exports its own" 'reexport_vs_local.spg:1' "'inc' is declared and exported here" 'reexport_vs_local.spg:3'
err reexport_duplo.spg          "'inc' is re-exported more than once" 'reexport_duplo.spg:2' "'inc' is first re-exported here" 'reexport_duplo.spg:1'
err reexport_double_list.spg    "'inc' is re-exported more than once" "'inc' is listed twice in this re-export"
err reexport_not_becoming_local.spg "undefined function 'inc'"
err ns_old_name.spg      "'sqrt' is no longer a global function; use '@math.sqrt'"
err ns_clock_antigo.spg     "'clock' is no longer a global function" '@time.unix()' '@time.elapsed()'
err ns_reservado_let.spg    "'@math' is a reserved namespace and cannot be used as a name" '(line 1, column 5)'
err ns_reservado_fn.spg     "'@time' is a reserved namespace" '(line 1, column 10)'
err ns_reservado_param.spg  "'@array' is a reserved namespace"
err ns_reservado_struct.spg "'@str' is a reserved namespace" '(line 1, column 8)'
err ns_no_member.spg       "'@math' is a namespace; use '@math.<name>(...)'"
err ns_membro_inexistente.spg "namespace '@math' has no member 'foo'"
err ns_desconhecido.spg     "unknown namespace '@mth'" '(available: @str, @array, @map, @into, @math, @sys, @task, @file, @json, @time, @char)' 'line 1, column 7'
err ns_arroba_solto.spg     "'@' must be followed by a namespace name" 'line 1, column 14'
err ns_sintaxe_antiga.spg   "'math.sqrt' uses the old namespace spelling" "use '@math.sqrt'"
err ns_string_antigo.spg    "'string.len' uses the old namespace spelling; use '@str.len'"
err ns_transform_antigo.spg "'transform.parseInt' uses the old namespace spelling" "use '@into.int'"
err ns_time_run_antigo.spg  "'time.run' uses the old namespace spelling" "use '@time.elapsed'"
err ns_time_run_removido.spg "namespace '@time' has no member 'run'"
err chamada_campo.spg       "'x' is a field of struct 'P', not a method" 'line 3'
err not_callable_expr.spg   "only functions can be called, and this value is not a function" 'line 2'
err ns_sleep_negativo.spg   '@time.sleep expects a non-negative number of milliseconds, got -5'
err into_int_invalido.spg   'cannot parse "abc" as int'
err into_float_invalido.spg 'cannot parse "abc" as float'
err into_int_vazio.spg      'cannot parse "" as int'
err pi_negativo.spg         '@math.pi expects a non-negative number of decimal places, got -1'
err e_negativo.spg          '@math.e expects a non-negative number of decimal places, got -3'
err pi_float.spg            "'@math.pi' argument #1 expects int, got float"
err into_bool.spg           "'@into.int' argument #1 expects float, got bool"
err ns_str_type.spg         "'@str.toUpper' argument #1 expects string, got int"
err opt_div_zero.spg        'division by zero'
err opt_index_oob.spg       'index out of bounds: 7 (array has 3 element(s))'

# diag <file> <expected error count> <fragment that MUST appear> [fragment that must NOT appear]
# Counts the errors (1 semantic error = 1 diagnostic) and forbids leaking
# internal format: mangled names ('#'/'::') and Rust Debug output (Named(/Array().
diag() {  # diag <file> <error count> <fragment> [<fragment>...] — ALL must appear
    f="$1"; n_esperado="$2"; shift 2
    raw=$(spgrun "diag/$f" 2>&1); code=$?
    got=$(echo "$raw" | tr '\\' '/')   # Windows prints lib\x.spg: compare with /
    n=$(echo "$got" | grep -c "^  type error")
    bad=$(echo "$got" | grep -E 'Named\(|Array\(|[a-z]#[0-9a-f]+::|::' | head -1)
    miss=""
    for want in "$@"; do echo "$got" | grep -qF -- "$want" || miss="$want"; done
    if [ "$code" = 1 ] && [ "$n" = "$n_esperado" ] && [ -z "$miss" ] && [ -z "$bad" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL diag/$f (exit=$code, errors=$n, expected=$n_esperado)"; [ -n "$bad" ] && echo "  vazou: $bad"; [ -n "$miss" ] && echo "  faltou: $miss"; echo "  got: $got"; fi
}

diag dup_field.spg      1 "struct 'P' has no field 'y'"
diag dup_undef.spg      1 "undefined variable 'doesNotExist'"
diag dup_op.spg         1 "operator Add not supported for int and string"
diag ret_types.spg      2 "function returns int but this return gives string"
diag clean_type.spg     1 "declared type is array<Point> but value is int"
diag mangled_struct.spg 1 "declared type is int but value is Point"
diag mangled_campo.spg  1 "struct 'Point' has no field 'y'"
diag mangled_func.spg   1 "'two' expects 0 argument(s), got 2"
diag ns_type.spg        1 "'@math.sqrt' argument #1 expects float, got int"

# the bug that motivated the patch: two DIFFERENT files with an error at the
# SAME line/column were indistinguishable (just "line 1, column 31" twice
# times) — now each message names its own file
diag ambiguous.spg 2 "type error in lib/liba.spg at line 1, column 31" "type error in lib/libb.spg at line 1, column 31"
# the ENTRY file of a multi-file program must also
# appear by name, not only the imported modules
diag entry_with_error.spg 2 "type error in lib/liba.spg" "type error in diag/entry_with_error.spg at line 2"
# methods: the struct/method name shown to the user is the one they wrote (no module tag)
diag method_mangled_arity.spg    1 "'Vec.add' expects 1 argument(s), got 0"
diag method_mangled_unknown.spg  1 "struct 'Vec' has no method 'nope'"
diag method_mangled_notcalled.spg 1 "'len2' is a method of struct 'Vec', not a field"
# an element of an array literal must also be visited by the
# file annotation (a real finding during development: the
# first version only stamped the Pos of the ArrayLit, not each item's)
diag arraylit_error.spg 1 "type error in lib/array_literal_error.spg" "undefined variable 'doesNotExist'"

# enums: `enum E { A, B }`, `E.A`, == / != only, `for x in E`, printed by variant name
ok enum_basic.spg  'Green\ncolor: Green Green\ntrue false false\nRed hot\nGreen natural\nBlue cool\n[Blue, Red]\nCar { name: fusca, color: Blue }\ntrue false\n3 Red'
ok enum_forms.spg  '101 10\n1 10 100\ntrue false\nGreen Red true\ntrue false\ntrue false\ntrue\nfalse Red\nSignal { now: Green, ticks: 1 } true\n[Yellow, Red, Yellow] 1 -1\n2'
ok match_basic.spg  'moving down\n210\nhorizontal vertical\none few many\nelif ok\nnot up'
ok enum_multi.spg  '0 Red\n1 Green\n2 Blue\n3 Red\ntrue\nRed\nGreen\nBlue\nMagenta'
err enum_bad_variant.spg    "enum 'Color' has no variant 'Purple' (variants: Red, Green)" "line 2"
err enum_bare_name.spg      "'Color' is an enum, not a value" "for x in Color"
err enum_lt.spg             "operator Lt not supported for Color and Color"
err enum_plus.spg           "operator Add not supported for Color and Color"
err enum_vs_int.spg         "operator Eq not supported for Color and int"
err enum_two_enums.spg      "operator Eq not supported for Color and Size"
err enum_field.spg          "type Color has no fields"
err enum_method.spg         "type Color has no methods"
err enum_empty.spg          "enum 'E' needs at least one variant"
err enum_dup.spg            "enum 'E' already has a variant named 'A'" "line 1, column 16"
err enum_missing_comma.spg  "expected '}'"
err enum_intof.spg          "'@into.int' argument #1 expects float, got Color"
err enum_assign_int.spg     "cannot assign int to 'c'"
err enum_annot.spg          "declared type is int but value is Color"
diag enum_mangled_variant.spg 1 "enum 'Color' has no variant 'Purple'"
diag enum_mangled_bare.spg    1 "'Color' is an enum, not a value"
diag enum_mangled_cmp.spg     1 "operator Eq not supported for Color and Color"

# install "file.spg": every export, lowest priority; a clash between two installs only fails when the name is USED
ok install_basic.spg          '15 1 Slow\nFast\nSlow'
ok install_priority.spg       '2 77 20'
ok install_unused_clash.spg   '30'
err install_clash_used.spg    "'helper' is installed from both" "line 1" "line 2" "import it by name"
err install_private.spg       "undefined function 'hidden'"
err install_no_ext.spg        "must end in '.spg'"

# ---- map<K, V> ----
ok map_basic.spg  '[ana: 31, bia: 26, caio: 41]\n31 3 true false\n-1 26\n[ana, bia, caio] [31, 26, 41]\ntrue false\n[ana: 31, caio: 41] 72\n[ana, caio, bia]\n[:] 0 0\n[1: c, 2: b]'
ok map_forms.spg  'g\n[1: [1, 2], 2: [99]]\n6\n4\n2\n3.0\n[5: 2, 10: 7]\ntrue'
ok map_multi.spg  '3 axe 2\n2\n9'
ok into_any.spg       '[a, b]\n[1, 2, 3]\n[ana: 1]\nP { x: 1, y: 2 }\nGreen\ntrue 3 2.5\ntext!\n6'
ok type_builtin.spg       'int float bool string\narray map Point Circle\nColor function none\nstring none\ntrue false true\ntexto hi'
ok union_params.spg           'checked string\nchecked float\nhit\nmiss\nhit\nnone\nvalue string\nvalue float\nstring'
ok array_expected.spg         '3\n3\n0\n2\n2\n2\n2\n2\ntrue'
ok field_map_expected.spg     '2\n2\n2\n2'
ok into_methods.spg   '[a, b] 42 2.5 true Green\n18 7.0 10.5\nP { x: 1, y: 2 } 1'
err map_float_key.spg       "map keys must be int, string, bool or an enum, found float" "line 2, column 8"
err map_key_mismatch.spg    "this map's keys are string, found int" "line 2, column 8"
err map_value_mismatch.spg  "this map's values are int, found string" "line 2, column 8"
err map_lit_keys.spg        "this map's keys are string, found int" "line 1, column 9"
err map_lit_values.spg      "this map's values are int, found string" "line 1, column 9"
err map_untyped_empty.spg   "cannot tell what '[:]' holds" "let m: map<string, int> = [:]"
err map_get_fallback.spg    "'@map.get' argument #3 expects int, got string"
err map_not_a_map.spg       "'@map.len' argument #1 expects map<any, any>, got array<int>"
err map_missing_key.spg     'key not found in map: "zz"' "@map.has"
err map_keysof_type.spg      "'@map.keysOf' argument #2 expects int, got string" "line 2, column 18"
err let_void.spg             "this expression has no value (void)" "line 1, column 1"
err const_void.spg           "this expression has no value (void)" "line 1, column 1"
err when_nonexhaustive.spg "does not cover every variant" "missing: Left, Right" "line 3, column 1"
err when_case_type.spg     "this case is string, but the matched value is Dir" "line 4, column 5"
err when_subject_float.spg "can only match an int, a string or an enum, but this value is float"
err into_method_int.spg     "'@into.int' argument #1 expects float, got int" "line 2, column 12"
err map_missing_compound.spg 'key not found in map: "zz"'
err map_parse_colon.spg     "expected ']'" "line 1, column 14"
diag map_mangled.spg 2 "this map's keys are string, found int" "this map's values are int, found Item"

# ---- enum with values, array == array, empty [] ----
ok enum_values.spg    '2 1 false true\n0 10 11 12\nbeta false 1.5 -1.5\n-5 -4 2\n12 High High\nA alfa\nB beta\n200'
ok array_eq_empty.spg 'true false false true true\ntrue false\n[5] true []\n[] 0\ntrue false true\ntrue true'
err enum_val_dup_name.spg "enum 'Ns' already has a variant named 'one'" "line 1, column 20"
err enum_val_mixed.spg    "variant 'B' has a string value but the earlier variants are int" "line 1, column 17"
err enum_val_needed.spg   "variant 'B' needs a value" "only int enums are numbered automatically" "line 1, column 19"
err enum_val_notlit.spg   "expected a literal value for the variant" "line 2, column 14"

# ---- @array / @math completos (insert, remove, clear, fill, concat, sum; trigonometria, exp, log, sign, clamp, tau) ----
ok ns_array_math.spg  '[10, 15, 20, 30, 99]\n10 99\n[15, 20, 30] 65 4.0 10\n[15, 20, 30, 7, 8] [1, 15, 20, 30] [15, 20, 30]\n[0, 0, 0, 5] [ab, ab] [] 5\n[] 0 0\n[y, x] [y, x, y, x] true 1\n0.0 1.0 0.0 1.0 0.0\n3 3 3142\n3142 0.0 0.0\n180 3142\n6.28 6.2832 -1 0 1 -1\n10 0 5 1.0\n-inf NaN'
err array_push_type.spg        "'@array.push' argument #2 expects int, got string" "line 2, column 12"
err array_insert_type.spg      "'@array.insert' argument #3 expects int, got bool"
err array_fill_struct.spg      "@array.fill repeats ONE value" "found P"
err array_concat_mismatch.spg  "'@array.concat' argument #2 expects array<int>, got array<string>"
err array_remove_oob.spg       "index out of bounds: 2 (array has 2 element(s))"
err array_fill_negative.spg    "@array.fill needs a count of 0 or more, got -1"
err math_clamp_bounds.spg      "the lower bound (5) is greater than the upper bound (2)"

# ---- full @str (pad, lastIndexOf, trimStart/End), empty pop, homogeneous array literal ----
err array_pop_empty.spg       "@array.pop on an empty array"
err array_lit_mixed_num.spg   "all items of an array must have the same type: expected int, found float" "line 1, column 9"
err array_lit_mixed_type.spg  "expected string, found int"
err str_pad_type.spg          "'@str.padStart' argument #2 expects int, got string"

# ---- native methods: s.len(), xs.push(1), m.has(k) = @str/@array/@map with the value first ----
ok native_methods.spg  '[7, 3, 1, 2, 10] 5 true 2 23\n10 7 [3, 1, 2]\nNAÏVE 3 3 true 005\n3 4 [9, 2, 1, 3]\ntrue 1 -1 [a] true 0\n[x, y] 2 x-y\n[r: [1, 2]] 2 1\ntrue true'
err native_no_method.spg     "type string has no method 'nope'" "@str functions that take it first"
err native_not_receiver.spg  "type array<int> has no method 'range'"
err native_arity.spg         "'@array.push' expects 1 argument(s), got 0"
err native_argtype.spg       "'@array.push' argument #1 expects int, got string"
err native_map_key.spg       "'@map.has' argument #1 expects string, got int"

# ---- throw / try / catch (exceptions across functions, catchable runtime errors, break/continue leaving a try) ----
ok throw_catch.spg  '10\ncaught: negative: -3\nmid-expression: negative: -1 12\nbottom\nafter stack ok 3\ninner+rethrown\nloop done 30\nafter loop\npost-while 2\nrt: division by zero\nrt: index out of bounds: 5 (array has 1 element(s))\nrt: key not found in map: "zz" (use @map.has(m, k) or @map.get(m, k, fallback))\nrt: cannot parse "abc" as int\nrt: @array.pop on an empty array\n8 -1\nf2 always\n7\nwrapped(always)\nend'
err throw_uncaught.spg             "unhandled exception: boom"
err throw_in_function_uncaught.spg "unhandled exception: deep"
err throw_not_string.spg           "'throw' takes a string (the message), found int"
err catch_type.spg                 "'catch' receives the thrown string" "not int"

# ---- @file / @sys: real files (temporary directory), exit code, arguments after `--`, stdin ----
check() {   # check <label> <expected code> <expected output> <actual code> <actual output>
    if [ "$4" = "$2" ] && [ "$5" = "$3" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL $1 (exit=$4, expected $2)"; echo "  want: $(echo "$3" | tr '\n' '|')"; echo "  got:  $(echo "$5" | tr '\n' '|')"; fi
}
tmpd=$(mktemp -d)
got=$(spgrun sys/file_ops.spg -- "$tmpd" 2>&1); code=$?
check sys/file_ops 0 "$(printf 'false true\n19 [one, two, three, four] 4\n[a.txt, sub] [c.txt, fundo] true\n[]\ncaught: true\nagain: true\nwrite fails: true\n1 true true fallback')" "$code" "$got"
got=$(spgrun sys/file_lines.spg -- "$tmpd" 2>&1); code=$?
check sys/file_lines 0 "$(printf '[a, b, c]\n6\nfallback')" "$code" "$got"
rm -rf "$tmpd"
tmpd=$(mktemp -d)
got=$(spgrun sys/file_meta.spg -- "$tmpd" 2>&1); code=$?
check sys/file_meta 0 "$(printf '5 [0, 255, 10, 13, 65]\ntrue true 20\n6 6\nnon-empty dir refuses delete\nfalse\nfalse\n@file.writeBytes: the item at index 1 is 300; bytes go from 0 to 255\nfalse\ndir: true\nghost: true\n0 []')" "$code" "$got"
rm -rf "$tmpd"
got=$(spgrun sys/exit_code.spg -- a "b c" --time 2>&1); code=$?
check sys/exit_code 7 "before [a, b c, --time]" "$code" "$got"
got=$(printf 'ana\nbia\r\nresto\nmais\n' | spgrun sys/stdin.spg 2>&1); code=$?
check sys/stdin 0 "$(printf 'name?\nhi ana 3\n11\nend: @sys.readLine: end of input (nothing left on stdin)')" "$code" "$got"
got=$(printf 'Ana\n30\n' | spgrun sys/readline_prompt.spg 2>&1); code=$?
check sys/readline_prompt 0 "$(printf 'Name: Age: Hi Ana 30\nend: readLine: end of input (nothing left on stdin)')" "$code" "$got"
err file_read_missing.spg "runtime error: @file.read(\"this/does/not/exist.txt\"): no such file or directory"
err sys_exit_type.spg     "'@sys.exit' argument #1 expects int, got string"
err file_arg_type.spg     "'@file.exists' argument #1 expects string, got int"
err file_bytes_type.spg   "'@file.writeBytes' argument #2 expects array<int>, got array<string>"

# the optimizer must never change behaviour: every program in the project
# has to print the same thing (and exit the same way) with and without it
for f in ok/*.spg lang/*.spg ../examples/*.spg; do
    # environment-dependent programs: their output depends on the machine (time, PowerShell on the PATH),
    # not on the optimizer. ps_missing is checked on its own below, with PATH controlled.
    case "$f" in *time_test*|*sleep*|*ps_missing*) continue;; esac
    a=$(spgrun "$f" 2>&1); ca=$?
    b=$(spgrun --no-opt "$f" 2>&1); cb=$?
    if [ "$a" = "$b" ] && [ "$ca" = "$cb" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL same/$f: optimized and --no-opt differ"; fi
done


# ---- file directives (#> strict / #> timeout N / #> friendly-terminal) ----
exit124() {  # exit124 <file> <expected output before the cut> — the engine ITSELF cuts with exit 124 and the message
    # (the suite's `timeout 30` also gives 124 on a hang: without the engine's message this is a FAIL)
    raw=$(spgrun "err/$1" 2>&1); code=$?; got=$(printf '%s' "$raw" | norm)
    if [ "$code" = 124 ] && echo "$got" | grep -qF "timeout: the program ran for more than 300 ms" && echo "$got" | head -1 | grep -qF -- "$2"; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL err/$1 (exit=$code)"; echo "  got: $(echo "$got" | tr '\n' '|')"; fi
}
ok ui_events.spg 'closed\n-1.0\n1\nbye\n-1.0'
ok ui_mouse.spg '-1.0\nfalse\nfalse\nfalse'
ok ui_shape_clicks.spg 'false\ntrue\nfalse\nfalse\nfalse'
ok ui_text.spg 'Olá\n16\nOi\ndone'
ok ui_text_size.spg '48.0\n16.0\n[]'
ok ui_text_wrap.spg '16.0\n24.0\ntrue'
ok ui_objects.spg '8.5\nfalse\n20\n1052688\n0.0\nkilled'
err ui_readonly.spg "cannot assign to 'Window.width'"
err ui_no_method.spg "has no method 'fly'"
err ui_use_after_kill.spg "this shape was killed"
err ui_handler_params.spg "expects function() -> void"
err ui_resize_params.spg "expects function(int, int) -> void"
err ui_native_name.spg "is also the name of a type of install #ui"
err ui_mouse_readonly.spg "cannot assign to 'Window.mouseX'"
err ui_mouse_string.spg "expects Mouse"
err ui_hovered_readonly.spg "cannot assign to 'Shape.hovered'"
err ui_text_type.spg "expects string"
err ui_text_size_range.spg "the text size must be between 1 and 512 pixels"
err ui_align_type.spg "expects Align"
err ui_wrap_negative.spg "the wrap width must be 0 (no wrap) or more pixels"
err ui_compound_call.spg "can not be changed with a compound operator here"
ok dir_strict_overflow_try.spg  'caught'
ok dir_timeout_ok.spg           'rapido'
ok dir_strict_siblings.spg        '4'
ok dir_strict_return_if.spg     '1'
err dir_strict_overflow.spg     'integer overflow in multiplication'
err dir_strict_shadow.spg       "'a' hides a name from an outer scope"
err dir_strict_unreachable.spg  "code after 'return' can never run"
err dir_unknown.spg             "did you mean '#> strict'?"
err dir_misplaced.spg           'directives must come before any code'
err dir_timeout_bad.spg         'expects just the number of milliseconds'
err dir_duplicate.spg           "directive '#> strict' is written twice"
err dir_friendly_lex.spg        'Oops! Unterminated string' 'Check for a missing closing quote'
err dir_friendly_strict.spg     'Oops! I found 1 problem(s) with the types' 'Rename the inner one'
err dir_in_import.spg           'only the file you run'
exit124 dir_timeout_loop.spg    'before'
exit124 dir_timeout_sleep.spg   ''


# ---- @json ----
ok json_basic.spg  'spg\n3\ntrue\nb\n1.5\nnull\n["a","b"]\narray\nnull\ntrue\nfalse\n2\n5\n[x, z]\ntrue\nfalse\n4\nno q'
ok json_make.spg   '{"name":"lamp","qty":2,"price":9.5,"kind":"Toy","tags":["a","b"]}\n{"a":1,"b":2}\n[1,2,3]\n"hi"\n1.0\n{\n  "name": "lamp",\n  "qty": 2,\n  "price": 9.5,\n  "kind": "Toy",\n  "tags": [\n    "a",\n    "b"\n  ]\n}'
ok methods_numeric.spg '7\n2.5\n3\n9\n2.5\n9.5\nNaN\n4.0\n3\n1.0\n0.7853981633974483\n1\n9\n-1\n-7.0\n-2\n-7\n-2.5\ntrue\n5\nNAÏVE\n3\n6\na-b'
err json_missing.spg  "@json.get: path not found: 'b'" "has no key 'b'"
err json_invalid.spg  'invalid JSON: expected a string key' 'line 1, column'
err json_friendly.spg 'Oops! @json.get: path not found'

err interp_vazia.spg     'empty interpolation' 'line 1, column 7'
err interp_type.spg     'Add not supported for int and string'

exact1 trace_cadeia.spg '6\nruntime error: division by zero\n  at divide (err/trace_cadeia.spg:2)\n  at compute (err/trace_cadeia.spg:6)\n  at <main> (err/trace_cadeia.spg:11)'
exact1 trace_recursao.spg 'runtime error: stack overflow: more than 100000 nested calls (is a function calling itself forever?)\n  at down (err/trace_recursao.spg:2)\n  ... same line repeated 49 more times\n  ... 99950 more frames'
exact1 trace_modulo.spg 'error: Oops! index out of bounds: 9 (array has 3 element(s)). The program stopped while running. Check the values involved at that point; a try/catch can handle this kind of error.\n  at risky (err/trace_modulo_lib.spg:3)\n  at outer (err/trace_modulo.spg:5)\n  at <main> (err/trace_modulo.spg:8)'
ok trace_catch.spg 'caught\nend'
ok lambdas.spg           '13\n42\n49\n10\n101\n20\niife\n<function>'
ok lambda_captura.spg    '203\n10'
err lambda_captura_atribui.spg "a lambda cannot assign 'x'"
err lambda_type_argument.spg "'ap' argument #1 expects function(int) -> int, got function(string) -> int"
err lambda_no_return.spg "function '<lambda>' must return int on every path"
exact1 trace_lambda.spg 'runtime error: division by zero\n  at t (err/trace_lambda.spg:1)\n  at <lambda> (err/trace_lambda.spg:2)\n  at <main> (err/trace_lambda.spg:3)'

ok optional_forms.spg          '1 none\n8 -1\n99 10\ntrue true true false\n6 0\nhi Ana\nnone and 10\n[1, none, 3]\n1\n5 0 5 0\n10 0\n7 1'
ok opt_global_catch.spg  'caught'
ok defaults.spg '10 hi -1.5 true\n3 hi -1.5 true\n3 x -1.5 true\n3 x 2.0 true\n3 x 2.0 false\n8\n3'
err opt_aritmetica.spg   'operator Add not supported for int? and int' 'may be none'
err opt_none_no_type.spg "cannot tell which type 'none' belongs to"
err opt_to_int.spg     "argument #1 expects int, got int?"
err opt_coalesce.spg     "'??' needs an optional value on its left"
err opt_no_output.spg    'operator Add not supported for int? and int'
err opt_atribui_none.spg 'operator Add not supported for int? and int'
err opt_campo.spg        'type N? has no fields'
err opt_void.spg         'void? makes no sense'
err match_dup_case.spg     'already has a case for'
err match_dup_enum.spg     'already has a case for'

# spg build: the executable runs its program from the embedded sources alone,
# so it must work after the source files are gone
bdir=$(mktemp -d)
cp ok/interpolation.spg ok/subpastas.spg "$bdir"/ 2>/dev/null
if timeout 60 "$SPG" build ok/interpolation.spg -o "$bdir/app" >/dev/null 2>&1; then
    got=$(cd / && "$bdir/app" 2>&1); code=$?
    want=$(printf 'hi Ana, I am 30 years old\nsum 31 and 3 items\nnested: xAnay / 1\nraw ${name} and $5 and 6\n1.5|true|[1, 2, 3]\n3')
    if [ "$code" = 0 ] && [ "$got" = "$want" ]; then pass=$((pass+1)); tick; else
        fail=$((fail+1)); tick; say "FAIL build/interpolation (exit=$code)"; echo "  got: $got"; fi
else
    fail=$((fail+1)); tick; say "FAIL build: spg build did not produce an executable"
fi
rm -rf "$bdir"

# spg fmt: rewrites the layout, keeps the comment, and a formatted file passes --check
fdir=$(mktemp -d)
printf 'let a=1+2\nprint(a)   // sum\n' > "$fdir/x.spg"
spgrun fmt "$fdir/x.spg" >/dev/null 2>&1
got=$(cat "$fdir/x.spg"); want=$(printf 'let a = 1 + 2\nprint(a)  // sum')
if [ "$got" = "$want" ] && spgrun fmt --check "$fdir/x.spg" >/dev/null 2>&1; then pass=$((pass+1)); tick; else
    fail=$((fail+1)); tick; say "FAIL fmt/x.spg"; echo "  got: $got"; fi
rm -rf "$fdir"

# ps.* (install #ps): the no-PowerShell path (runs with an empty PATH, so it works the same on every machine)
raw=$(timeout 30 env PATH=/nonexistent "$SPG" ok/ps_missing.spg </dev/null 2>&1); code=$?
want=$(printf 'false\nps.run: PowerShell not found (tried pwsh and powershell)')
if [ "$code" = 0 ] && [ "$raw" = "$want" ]; then pass=$((pass+1)); tick; else
    fail=$((fail+1)); tick; say "FAIL ok/ps_missing.spg (exit=$code)"; echo "  got: $raw"; fi

raw=$(timeout 30 env PATH=/nonexistent "$SPG" ok/ps_safe.spg </dev/null 2>&1); code=$?
want='ps.runFile: blocked by #> safe-party: runFile only takes a .ps1 inside the script'"'"'s folder (no absolute paths, no ..)'
if [ "$code" = 0 ] && [ "$raw" = "$want" ]; then pass=$((pass+1)); tick; else
    fail=$((fail+1)); tick; say "FAIL ok/ps_safe.spg (exit=$code)"; echo "  got: $raw"; fi
err ps_sem_install.spg "'ps' is an installable library, not a variable: it needs \`install #ps\`"
err ps_arroba.spg "'@ps' is not built in"

# async: `-> task<T>` runs eagerly, `.wait()` reads the result or re-throws the error
ok async_basic.spg "double started with 21\ncreated, not waited yet\n<task>\n43\ncaught: too big\n1"
err async_wait_trace.spg "division by zero" "at divide (" "before wait"
ok async_overlap.spg "a starts\nb starts\nboth started\nb done\na done\nab"
ok async_many.spg "89700"
ok async_all.spg "b done\nc done\na done\n[a, b, c]\n3\n[]"
ok async_lambda.spg "say: hi\nafter\n[11, 12]"
ok async_unwaited.spg "nobody waits on t\nwarning: a task failed and nobody called .wait() on it: unhandled exception: boom\n  at bad (ok/async_unwaited.spg:3)"
err async_all_error.spg "unhandled exception: boom" "at bad ("
ok pipe_basic.spg "6\n6\n13\n6\n7\n106"
err pipe_no_call.spg "expected a function call after the pipe"
ok logic_words.spg "true\ntrue\nfalse"
err http_refused.spg "http.get: io: Connection refused"
err http_timeout_zero.spg "the timeout must be at least 1 second"
ok static_basic.spg "0 0 3 4"
err static_on_value.spg "is an associated function of"
err static_missing.spg "has no associated function"
err static_unknown_type.spg "needs a struct named"
err static_with_me.spg "associated functions have none"
ok static_enum.spg "true true true"
ok const_alias.spg          '1\n1'
ok const_copies_nested.spg  '1'
ok const_catchable.spg      'caught: cannot modify a const value (it is read-only); use .copy() to change a copy\n1'
ok copy_array.spg            '3\n2'
ok copy_struct.spg           '1\n9'
ok copy_map.spg              '2\n1'
ok copy_nested.spg           '1\n5'
ok copy_user_method.spg      '30'
err static_enum_clash.spg "clashes with the variant"
err static_enum_on_value.spg "is an associated function of"
err static_enum_missing.spg "has no associated function"
err const_push.spg 'cannot modify a const value'
err const_nested_write.spg 'cannot modify a const value'
err const_struct_field.spg 'cannot modify a const value'
err const_map_write.spg 'cannot modify a const value'
err const_param_mutates.spg 'cannot modify a const value'
err const_return_mutable.spg 'cannot modify a const value'
err const_jit_loop.spg 'cannot modify a const value'
err const_cycle_field.spg 'cannot modify a const value'
err const_friendly.spg 'Oops! cannot modify a const value (it is read-only). Make a copy with .copy() and change that copy instead.'
err friendly_undefined.spg 'Check the spelling of the name, and declare it before it'
err friendly_parse_eof.spg 'but I found the end of the file'
err friendly_parse_else.spg 'needs an '"'"'else'"'"' too'
err friendly_parse_token.spg 'but I found '"'"'='"'"'.'

say "multi-file: $pass ok, $fail failed"
[ "$fail" = 0 ]