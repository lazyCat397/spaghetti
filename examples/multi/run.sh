#!/bin/bash
# Regressão do multi-arquivo. Confere a SAÍDA EXATA (não só o exit code):
# um bug de renomeio costuma dar resultado errado sem crashar.
# Uso (a partir de spaghetti-skeleton/): bash examples/multi/run.sh
cd "$(dirname "$0")"
SPG=../../target/release/spg
spgrun() { timeout 30 "$SPG" "$@"; }   # a hung program (infinite loop) becomes a clean FAIL (exit 124), not a stuck suite
pass=0; fail=0

ok() {   # ok <arquivo> <saída esperada, com \n>
    got=$(spgrun "ok/$1" 2>&1); code=$?
    want=$(printf "$2")
    if [ "$code" = 0 ] && [ "$got" = "$want" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL ok/$1 (exit=$code)"; echo "  want: $(echo "$want" | tr '\n' '|')"; echo "  got:  $(echo "$got" | tr '\n' '|')"; fi
}
err() {  # err <arquivo> <trecho> [<trecho>...]  — TODOS devem aparecer
    f="$1"; shift
    got=$(spgrun "err/$f" 2>&1); code=$?
    miss=""
    for want in "$@"; do echo "$got" | grep -qF -- "$want" || miss="$want"; done
    if [ "$code" = 1 ] && [ -z "$miss" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL err/$f (exit=$code)"; echo "  faltou: $miss"; echo "  got: $got"; fi
}

ok basico.spg            '42\n6.28\n0\n7\n42'
ok diamante.spg          '120\n120'
ok mesmo_nome.spg        '10'
ok structs.spg           '11\n21\n4\n6\n77\n40'
ok shadowing.spg         '6\n7\n500\n1000\n500'
ok builtin_homonimo.spg  '1001\n3'
ok subpastas.spg         'meio+folha'
ok reexport_simples.spg     '2\n7'
ok reexport_identidade.spg  '3'
ok reexport_cadeia.spg      '2'
ok reexport_mista.spg       '0\n100\n5'
ok reexport_privada_homonima.spg '999'
ok ns_modulo.spg         '5\nOI\ntrue'
ok ns_todos.spg          '5\nSPG\ntrue\n2\ntrue\n2\n4\na+b+c\n9\n2.5\n3\n42true\n5\n5\n124\nfalse\ntrue\ntrue\ntrue\n20\nT\ntrue'
ok ns_campos.spg         '7\nx'
ok ns_nomes_livres.spg   '6\n6\n4\nT'
ok ns_into.spg           '42\n5\n-5\n42\n7\n3\n3.5\n42true1.5\n3'
ok ns_pi_e.spg           '3.1416\n3\n3.1\n3.14\n3.14\n3.141592653589793\n3.141592653589793\n2.718\n2.72\n3\n3.14159'
ok str_chars.spg         '3\n3\n3\ná\nçã\nOLÁ\n3\ná|é|í\nééé\n3\ntrue\ntrue'

# buffered stdout (print goes through a 64 KB buffer when stdout is not a terminal).
# These prove ORDER and COMPLETENESS, which is what buffering could break.
exact1() {  # exact1 <arquivo> <saída esperada> — exit code 1 e saída (stdout+stderr) EXATA, em ordem
    got=$(spgrun "err/$1" 2>&1); code=$?
    want=$(printf "$2")
    if [ "$code" = 1 ] && [ "$got" = "$want" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL err/$1 (exit=$code)"; echo "  want: $(echo "$want" | tr '\n' '|')"; echo "  got:  $(echo "$got" | tr '\n' '|')"; fi
}
lines() {  # lines <arquivo> <nº de linhas> <última linha> — nada pode se perder além do buffer
    got=$(spgrun "ok/$1" 2>&1); code=$?
    n=$(echo "$got" | wc -l); last=$(echo "$got" | tail -1)
    if [ "$code" = 0 ] && [ "$n" = "$2" ] && [ "$last" = "$3" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL ok/$1 (exit=$code, linhas=$n, última=$last)"; fi
}
ok print_order.spg       'a\nE1\nb\nE2\nc'
lines print_big.spg 30000 29999
exact1 print_before_error.spg 'a\nE1\nb\nruntime error: division by zero'

# fused array-index ops (hand-verified values; the OOB errors must keep the exact message)
ok index_fused.spg       '[3, 2, 15, 7]\n[1.75, 3.25]\n[false, true, true]\n[[1, 12], [3, 3]]\n5\n9'
err index_fused_oob_addconst.spg 'index out of bounds: 9 (array has 3 element(s))'
err index_fused_oob_addlocal.spg 'index out of bounds: 9 (array has 3 element(s))'
err index_fused_oob_setlocal.spg 'index out of bounds: -1 (array has 3 element(s))'
err index_fused_oob_setconst.spg 'index out of bounds: 3 (array has 3 element(s))'
err index_fused_oob_set2d.spg    'index out of bounds: 5 (array has 2 element(s))'
err index_fused_oob_head.spg      'index out of bounds: 7 (array has 3 element(s))'

# builtins on the operand stack (call_builtin_on_stack) and the allocation-free charAt/substring (ASCII and UTF-8)
ok builtins_fast.spg     '5\n5\n3\n3\ntrue\nfalse\ntrue\nfalse\n[3, 1, 4, 1, 5, 9]\n9\n5\n[3, 1, 4, 1]\n7\n3.5\n-7\n4\n-3.5\n2.5\n-4\n-3\n3\n-4\n4\n-7\n-3\n4'
ok str_chars_utf8.spg    'a\nf\ná\no\nç\na\né\nc\ncd\ná mu\nolá mundo\naé\n[]\n[]\no\nl\ná\n \nm\nu\nn\nd\no\n9'
err charat_oob.spg          'index out of bounds: 3 (array has 3 element(s))'
err charat_neg.spg          'index out of bounds: -1 (array has 3 element(s))'
err charat_oob_utf8.spg     'index out of bounds: 3 (array has 3 element(s))'
err substring_oob_utf8.spg  'index out of bounds: 9 (array has 3 element(s))'
err substring_range.spg     'index out of bounds: 1 (array has 3 element(s))'
err substring_neg.spg       'index out of bounds: 2 (array has 3 element(s))'

# stdout buffering with the timed flusher thread: nothing lost or reordered under load...
# fused struct-field ops (hand-verified; includes reference semantics: `let u = s; u.a += 1` changes s)
ok field_fused.spg       '57\n14\n0.875\n7\n99\n0.5\n8\n297\n100\n200\n9.5\n305'

lines print_stress.spg 300000 299999
# ...and nothing held back: the first print must reach a pipe while the program is still crunching
# (the program is killed as soon as it shows up, so this takes ~15 ms)
if command -v python3 >/dev/null 2>&1; then
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
    then pass=$((pass+1)); else fail=$((fail+1)); echo "FAIL timing/flush_latency.spg: o 1o print nao chegou ao pipe enquanto o programa calculava"; fi
else echo "skip timing/flush_latency (sem python3)"; fi

# optimizer regressions (hand-verified expected values)
ok opt_jump_targets.spg  'true\nfalse\n13\n6\n8\n18\n1\n0\n1'
ok opt_loops.spg         '16\n0\n55\n0\n6\n1\n8\n0\n35\n12'
ok opt_ops.spg           '27\n13\n60\n5\n0\n60\n17\n37\n35\n15\n40\n-2\n-16\n-27\n-2\n-1\n-36\n-13\n-22\n-24\n-14\n-17\n11100\n1100101\n1101010\n6\n59\n4.25\n27\n50'

# '...' raw strings (no escapes, may hold "double quotes") and ranges: for i in a..b [step s] / @array.range
# (expected values verified by hand, line by line)
ok raw_strings.spg       'print("hi")\nhi\n15\na\\nb\n4\na\nb\n3\nolá\n\ntrue\n1\nabc\nmulti\nline'
ok range_for.spg         '1\n2\n3\n-\n0\n3\n6\n9\n- down\n5\n3\n1\n3\n2\n1\n- empty\n- non-literal step\n0\n3\n6\n7\n4\n1\n0\n3\n6\n9\n- continue / break\n1\n3\n5\n7\n9\n3\n- reassigning i does not move the counter\n10\n11\n12\n100\n101\n102\n- expressions as bounds, evaluated once\n0\n1\n2\n1\n5050\n10\n- loop variable shadows an outer one, bounds still see the outer\n0\n1\n2\n3\n- array.range\n[1, 2, 3, 4]\n[0, 3, 6, 9]\n[5, 3, 1]\n[]\n[]\n[]\n[-5, 0]\n[0]\n[9223372036854775800, 9223372036854775803, 9223372036854775806]\n0\n3\n6\n1000\n- extremes\n9223372036854775805\n9223372036854775806'
err range_step_zero.spg          'a range step cannot be zero' 'range_step_zero.spg at line 1'
err range_start_float.spg        'the start of a range must be int, found float'
err range_end_string.spg         'the end of a range must be int, found string'
err range_step_float.spg         'the step of a range must be int, found float'
exact1 range_step_zero_dynamic.spg 'before\nruntime error: range step cannot be zero'
err range_array_step_zero.spg    'runtime error: range step cannot be zero'
err range_array_too_large.spg    '@array.range would create 100000000 elements (the maximum is 20000000)' "use 'for i in a..b'"
err range_outside_for.spg        "'..' only works in a for loop header" "'@array.range(1, 4)'" '(line 1, column 10)'
err range_array_arity.spg        "'@array.range' expects 2 argument(s), got 1"
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
err method_on_int.spg         "type int has no methods (cannot call '.get()' on it)"
err method_not_called.spg     "'get' is a method of struct 'P', not a field; call it with '.get()'"
err method_field_clash.spg    "struct 'Q' has both a field and a method named 'x'" '(line 3, column 5)'
err method_me_param.spg       "'me' is already declared in this scope"
err method_duplicate.spg      "'Q.f' is declared more than once" 'method_duplicate.spg:6' "'Q.f' is first declared here"
err method_bad_field.spg      "struct 'Q' has no field 'zzz'"
err method_fields_need_comma.spg "expected '}', found Ident" 'line 2, column 12'
err method_return_mismatch.spg 'function returns int but this return gives string'

# print / printError take any number of arguments (values joined by one space; no arguments = an empty line)
ok print_variadic.spg    '\na\n1 2\nx 1 2.5 true raw "q"\n[1, 2] arr\n3 3\n1 2 3 4 5 6 7 8 9 10\n1 2 3\n3\n \n  \nonly \n\ne 1 false\nafter\n\nlast'

err ciclo_a.spg          'circular import'
err ciclo3_a.spg         'ciclo3_b.spg -> '
err auto_import.spg      'circular import'
err nao_exportado.spg    'is not exported'
err inexistente.spg      "has no declaration named 'fantasma'"
err sem_extensao.spg     "must end in '.spg'"
err arquivo_faltando.spg 'could not read'
err colisao_imports.spg  "'helper' is imported more than once" 'colisao_imports.spg:2' "'helper' is first imported here" 'colisao_imports.spg:1'
err colisao_local.spg    "'helper' conflicts with a local declaration" 'colisao_local.spg:1' "'helper' is also declared here" 'colisao_local.spg:2'
err colisao_local_dupla.spg "'a' is declared more than once" 'colisao_local_dupla.spg:3' "'a' is first declared here" 'colisao_local_dupla.spg:1'
err colisao_import_lista.spg "'dois' is imported more than once" "'dois' is listed twice in this import"
err reexport_import_nao_reexporta.spg "only imported by module" 'export { inc } from "./rx_origem.spg"'
err reexport_inexistente.spg    "has no declaration named 'fantasma'"
err reexport_privada.spg        "'privada' exists in module" "is not exported"
err reexport_sem_extensao.spg   "must end in '.spg'"
err reexport_ciclo.spg          'circular import' 'rx_ciclo_a.spg -> ' 'rx_ciclo_b.spg'
err reexport_vs_local.spg       "'inc' is re-exported here, but this file also exports its own" 'reexport_vs_local.spg:1' "'inc' is declared and exported here" 'reexport_vs_local.spg:3'
err reexport_duplo.spg          "'inc' is re-exported more than once" 'reexport_duplo.spg:2' "'inc' is first re-exported here" 'reexport_duplo.spg:1'
err reexport_duplo_lista.spg    "'inc' is re-exported more than once" "'inc' is listed twice in this re-export"
err reexport_nao_vira_local.spg "undefined function 'inc'"
err ns_nome_antigo.spg      "'sqrt' is no longer a global function; use '@math.sqrt'"
err ns_clock_antigo.spg     "'clock' is no longer a global function" '@time.unix()' '@time.elapsed()'
err ns_reservado_let.spg    "'@math' is a reserved namespace and cannot be used as a name" '(line 1, column 5)'
err ns_reservado_fn.spg     "'@time' is a reserved namespace" '(line 1, column 10)'
err ns_reservado_param.spg  "'@array' is a reserved namespace"
err ns_reservado_struct.spg "'@str' is a reserved namespace" '(line 1, column 8)'
err ns_sem_membro.spg       "'@math' is a namespace; use '@math.<name>(...)'"
err ns_membro_inexistente.spg "namespace '@math' has no member 'foo'"
err ns_desconhecido.spg     "unknown namespace '@mth'" '(available: @str, @array, @map, @into, @math, @time, @file, @sys, @json)' 'line 1, column 7'
err ns_arroba_solto.spg     "'@' must be followed by a namespace name" 'line 1, column 14'
err ns_sintaxe_antiga.spg   "'math.sqrt' uses the old namespace spelling" "use '@math.sqrt'"
err ns_string_antigo.spg    "'string.len' uses the old namespace spelling; use '@str.len'"
err ns_transform_antigo.spg "'transform.parseInt' uses the old namespace spelling" "use '@into.int'"
err ns_time_run_antigo.spg  "'time.run' uses the old namespace spelling" "use '@time.elapsed'"
err ns_time_run_removido.spg "namespace '@time' has no member 'run'"
err chamada_campo.spg       "'x' is a field of struct 'P', not a method" 'line 3'
err not_callable_expr.spg   "only named functions can be called" 'line 2'
err ns_sleep_negativo.spg   '@time.sleep expects a non-negative number of milliseconds, got -5'
err into_int_invalido.spg   'cannot parse "abc" as int'
err into_float_invalido.spg 'cannot parse "abc" as float'
err into_int_vazio.spg      'cannot parse "" as int'
err pi_negativo.spg         '@math.pi expects a non-negative number of decimal places, got -1'
err e_negativo.spg          '@math.e expects a non-negative number of decimal places, got -3'
err pi_float.spg            "'@math.pi' argument #1 expects int, got float"
err into_bool.spg           "'@into.int' argument #1 expects float, got bool"
err ns_str_tipo.spg         "'@str.toUpper' argument #1 expects string, got int"
err opt_div_zero.spg        'division by zero'
err opt_index_oob.spg       'index out of bounds: 7 (array has 3 element(s))'

# diag <arquivo> <nº de erros esperado> <trecho que DEVE aparecer> [trecho que NÃO pode aparecer]
# Conta os erros (1 erro semântico = 1 diagnóstico) e proíbe vazamento de
# formato interno: nome mangled ('#'/'::') e Debug do Rust (Named(/Array().
diag() {  # diag <arquivo> <nº de erros> <trecho> [<trecho>...] — TODOS devem aparecer
    f="$1"; n_esperado="$2"; shift 2
    got=$(spgrun "diag/$f" 2>&1); code=$?
    n=$(echo "$got" | grep -c "^  type error")
    bad=$(echo "$got" | grep -E 'Named\(|Array\(|[a-z]#[0-9a-f]+::|::' | head -1)
    miss=""
    for want in "$@"; do echo "$got" | grep -qF -- "$want" || miss="$want"; done
    if [ "$code" = 1 ] && [ "$n" = "$n_esperado" ] && [ -z "$miss" ] && [ -z "$bad" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL diag/$f (exit=$code, erros=$n, esperado=$n_esperado)"; [ -n "$bad" ] && echo "  vazou: $bad"; [ -n "$miss" ] && echo "  faltou: $miss"; echo "  got: $got"; fi
}

diag dup_field.spg      1 "struct 'P' has no field 'y'"
diag dup_undef.spg      1 "undefined variable 'naoExiste'"
diag dup_op.spg         1 "operator Add not supported for int and string"
diag ret_tipos.spg      2 "function returns int but this return gives string"
diag tipo_limpo.spg     1 "declared type is array<Ponto> but value is int"
diag mangled_struct.spg 1 "declared type is int but value is Ponto"
diag mangled_campo.spg  1 "struct 'Ponto' has no field 'y'"
diag mangled_func.spg   1 "'dois' expects 0 argument(s), got 2"
diag ns_tipo.spg        1 "'@math.sqrt' argument #1 expects float, got int"

# o bug que motivou o patch: dois arquivos DIFERENTES com um erro na
# MESMA line/column eram indistinguíveis (só "line 1, column 31" duas
# vezes) — agora cada mensagem nomeia seu próprio arquivo
diag ambiguo.spg 2 "type error in lib/liba.spg at line 1, column 31" "type error in lib/libb.spg at line 1, column 31"
# o arquivo de ENTRADA de um programa multi-arquivo também precisa
# aparecer nomeado, não só os módulos importados
diag entrada_com_erro.spg 2 "type error in lib/liba.spg" "type error in diag/entrada_com_erro.spg at line 2"
# methods: the struct/method name shown to the user is the one they wrote (no module tag)
diag method_mangled_arity.spg    1 "'Vec.add' expects 1 argument(s), got 0"
diag method_mangled_unknown.spg  1 "struct 'Vec' has no method 'nope'"
diag method_mangled_notcalled.spg 1 "'len2' is a method of struct 'Vec', not a field"
# elemento de um array-literal também precisa ser visitado pela
# anotação de arquivo (achado real durante o desenvolvimento: a
# primeira versão só carimbava a Pos do ArrayLit, não a de cada item)
diag arraylit_erro.spg 1 "type error in lib/rx_arraylit_erro.spg" "undefined variable 'naoExiste'"

# enums: `enum E { A, B }`, `E.A`, == / != only, `for x in E`, printed by variant name
ok enum_basic.spg  'Green\ncolor: Green Green\ntrue false false\nRed hot\nGreen natural\nBlue cool\n[Blue, Red]\nCar { name: fusca, color: Blue }\ntrue false\n3 Red'
ok enum_forms.spg  '101 10\n1 10 100\ntrue false\nGreen Red true\ntrue false\ntrue false\ntrue\nfalse Red\nSignal { now: Green, ticks: 1 } true\n[Yellow, Red, Yellow] 1 -1\n2'
ok enum_multi.spg  '0 Red\n1 Green\n2 Blue\n3 Red\ntrue\nRed\nGreen\nBlue\nMagenta'
ok enum_scope.spg  '7\nGreen\n1\n2\nRed'
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
ok map_forms.spg  'g\n[1: [1, 2], 2: [99]]\n6\n4\n2\n3\n[5: 2, 10: 7]\ntrue'
ok map_multi.spg  '3 axe 2\n2\n9'
err map_float_key.spg       "map keys must be int, string, bool or an enum, found float" "line 2, column 8"
err map_key_mismatch.spg    "this map's keys are string, found int" "line 2, column 8"
err map_value_mismatch.spg  "this map's values are int, found string" "line 2, column 8"
err map_lit_keys.spg        "this map's keys are string, found int" "line 1, column 9"
err map_lit_values.spg      "this map's values are int, found string" "line 1, column 9"
err map_untyped_empty.spg   "cannot tell what '[:]' holds" "let m: map<string, int> = [:]"
err map_get_fallback.spg    "'@map.get' argument #3 expects int, got string"
err map_not_a_map.spg       "'@map.len' argument #1 expects map<any, any>, got array<int>"
err map_missing_key.spg     'key not found in map: "zz"' "@map.has"
err map_missing_compound.spg 'key not found in map: "zz"'
err map_parse_colon.spg     "expected ']'" "line 1, column 14"
diag map_mangled.spg 2 "declared type is map<string, Item> but value is map<int, Item>" "this map's values are int, found Item"

# ---- enum com valores, array == array, [] vazio ----
ok enum_values.spg    '2 1 false true\n0 10 11 12\nbeta false 1.5 -1.5\n-5 -4 2\n12 High High\nA alfa\nB beta\n200'
ok array_eq_empty.spg 'true false false true true\ntrue false\n[5] true []\n[] 0\ntrue false true\ntrue true'
err enum_val_dup_name.spg "enum 'Ns' already has a variant named 'one'" "line 1, column 20"
err enum_val_mixed.spg    "variant 'B' has a string value but the earlier variants are int" "line 1, column 17"
err enum_val_needed.spg   "variant 'B' needs a value" "only int enums are numbered automatically" "line 1, column 19"
err enum_val_notlit.spg   "expected a literal value for the variant" "line 2, column 14"

# ---- @array / @math completos (insert, remove, clear, fill, concat, sum; trigonometria, exp, log, sign, clamp, tau) ----
ok ns_array_math.spg  '[10, 15, 20, 30, 99]\n10 99\n[15, 20, 30] 65 4 10\n[15, 20, 30, 7, 8] [1, 15, 20, 30] [15, 20, 30]\n[0, 0, 0, 5] [ab, ab] [] 5\n[] 0 0\n[y, x] [y, x, y, x] true 1\n0 1 0 1 0\n3 3 3142\n3142 0 0\n180 3142\n6.28 6.2832 -1 0 1 -1\n10 0 5 1\n-inf NaN'
err array_push_type.spg        "'@array.push' argument #2 expects int, got string" "line 2, column 12"
err array_insert_type.spg      "'@array.insert' argument #3 expects int, got bool"
err array_fill_struct.spg      "@array.fill repeats ONE value" "found P"
err array_concat_mismatch.spg  "'@array.concat' argument #2 expects array<int>, got array<string>"
err array_remove_oob.spg       "index out of bounds: 2 (array has 2 element(s))"
err array_fill_negative.spg    "@array.fill needs a count of 0 or more, got -1"
err math_clamp_bounds.spg      "the lower bound (5) is greater than the upper bound (2)"

# ---- @str completo (pad, lastIndexOf, trimStart/End), pop vazio, literal de array homogêneo ----
ok ns_str_pop.spg  '[007] [   ab] [abxyx] [ab   ]\nlong x ··é\n3 -1 3 1\n[hi  ] [  hi] [hi]\n3 2 [1] 1\n[[], [1]] [[1], []]'
err array_pop_empty.spg       "@array.pop on an empty array"
err array_lit_mixed_num.spg   "all items of an array must have the same type: expected int, found float" "line 1, column 9"
err array_lit_mixed_type.spg  "expected string, found int"
err str_pad_type.spg          "'@str.padStart' argument #2 expects int, got string"

# ---- métodos nativos: s.len(), xs.push(1), m.has(k) = @str/@array/@map com o valor na frente ----
ok native_methods.spg  '[7, 3, 1, 2, 10] 5 true 2 23\n10 7 [3, 1, 2]\nOLÁ 3 3 true 005\n3 4 [9, 2, 1, 3]\ntrue 1 -1 [a] true 0\n[x, y] 2 x-y\n[r: [1, 2]] 2 1\ntrue true'
err native_no_method.spg     "type string has no method 'nope'" "@str functions that take it first"
err native_not_receiver.spg  "type array<int> has no method 'range'"
err native_arity.spg         "'@array.push' expects 1 argument(s), got 0"
err native_argtype.spg       "'@array.push' argument #1 expects int, got string"
err native_map_key.spg       "'@map.has' argument #1 expects string, got int"

# ---- throw / try / catch (exceções entre funções, erros de runtime capturáveis, break/continue saindo de try) ----
ok throw_catch.spg  '10\ncaught: negative: -3\nmid-expression: negative: -1 12\nbottom\nafter stack ok 3\ninner+rethrown\nloop done 30\nafter loop\npost-while 2\nrt: division by zero\nrt: index out of bounds: 5 (array has 1 element(s))\nrt: key not found in map: "zz" (use @map.has(m, k) or @map.get(m, k, fallback))\nrt: cannot parse "abc" as int\nrt: @array.pop on an empty array\n8 -1\nf2 always\n7\nwrapped(always)\nend'
err throw_uncaught.spg             "unhandled exception: boom"
err throw_in_function_uncaught.spg "unhandled exception: deep"
err throw_not_string.spg           "'throw' takes a string (the message), found int"
err catch_type.spg                 "'catch' receives the thrown string" "not int"

# ---- @file / @sys: arquivos de verdade (diretório temporário), código de saída, argumentos após `--`, stdin ----
check() {   # check <rótulo> <código esperado> <saída esperada> <código obtido> <saída obtida>
    if [ "$4" = "$2" ] && [ "$5" = "$3" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL $1 (exit=$4, esperado $2)"; echo "  want: $(echo "$3" | tr '\n' '|')"; echo "  got:  $(echo "$5" | tr '\n' '|')"; fi
}
tmpd=$(mktemp -d)
got=$(spgrun sys/file_ops.spg -- "$tmpd" 2>&1); code=$?
check sys/file_ops 0 "$(printf 'false true\n20 [um, dois, tres, quatro] 4\n[a.txt, sub] [c.txt, fundo] true\n[]\ncaught: true\nagain: true\nwrite fails: true\n1 true true fallback')" "$code" "$got"
rm -rf "$tmpd"
tmpd=$(mktemp -d)
got=$(spgrun sys/file_meta.spg -- "$tmpd" 2>&1); code=$?
check sys/file_meta 0 "$(printf '5 [0, 255, 10, 13, 65]\ntrue true 20\n6 6\nnon-empty dir refuses delete\nfalse\nfalse\n@file.writeBytes: the item at index 1 is 300; bytes go from 0 to 255\nfalse\ndir: true\nghost: true\n0 []')" "$code" "$got"
rm -rf "$tmpd"
got=$(spgrun sys/exit_code.spg -- a "b c" --time 2>&1); code=$?
check sys/exit_code 7 "antes [a, b c, --time]" "$code" "$got"
got=$(printf 'ana\nbia\r\nresto\nmais\n' | spgrun sys/stdin.spg 2>&1); code=$?
check sys/stdin 0 "$(printf 'nome?\noi ana 3\n11\nfim: @sys.readLine: end of input (nothing left on stdin)')" "$code" "$got"
err file_read_missing.spg "runtime error: @file.read(\"this/does/not/exist.txt\"): no such file or directory"
err sys_exit_type.spg     "'@sys.exit' argument #1 expects int, got string"
err file_arg_type.spg     "'@file.exists' argument #1 expects string, got int"
err file_bytes_type.spg   "'@file.writeBytes' argument #2 expects array<int>, got array<string>"

# the optimizer must never change behaviour: every program in the project
# has to print the same thing (and exit the same way) with and without it
for f in ok/*.spg ../*.spg; do
    case "$f" in *time_test*|*sleep*) continue;; esac
    a=$(spgrun "$f" 2>&1); ca=$?
    b=$(spgrun --no-opt "$f" 2>&1); cb=$?
    if [ "$a" = "$b" ] && [ "$ca" = "$cb" ]; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL same/$f: optimized and --no-opt differ"; fi
done


# ---- file directives (#> strict / #> timeout N / #> friendly-terminal) ----
exit124() {  # exit124 <arquivo> <saída esperada antes do corte> — o PRÓPRIO engine corta com exit 124 e a mensagem
    # (o `timeout 30` da suíte também dá 124 numa trava: sem a mensagem do engine isso é FAIL)
    got=$(spgrun "err/$1" 2>&1); code=$?
    if [ "$code" = 124 ] && echo "$got" | grep -qF "timeout: the program ran for more than 300 ms" && echo "$got" | head -1 | grep -qF -- "$2"; then pass=$((pass+1)); else
        fail=$((fail+1)); echo "FAIL err/$1 (exit=$code)"; echo "  got: $(echo "$got" | tr '\n' '|')"; fi
}
ok dir_strict_overflow_try.spg  'pegou'
ok dir_wrap_default.spg         'wrap\n-9223372036854775808\n0'
ok dir_timeout_ok.spg           'rapido'
ok dir_strict_irmaos.spg        '4'
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
exit124 dir_timeout_loop.spg    'antes'
exit124 dir_timeout_sleep.spg   ''


# ---- @json ----
ok json_basic.spg  'spg\n3\ntrue\nb\n1.5\nnull\n["a","b"]\narray\nnull\ntrue\nfalse\n2\n5\n[x, z]\ntrue\nfalse\n4\nsem q'
ok json_make.spg   '{"name":"lamp","qty":2,"price":9.5,"kind":"Toy","tags":["a","b"]}\n{"a":1,"b":2}\n[1,2,3]\n"oi"\n1.0\n{\n  "name": "lamp",\n  "qty": 2,\n  "price": 9.5,\n  "kind": "Toy",\n  "tags": [\n    "a",\n    "b"\n  ]\n}'
ok json_set.spg    '{\n    "version": "0.15.0",\n    "items": [\n        1,\n        2,\n        3\n    ],\n    "author": "ana"\n}\n{"a":2}\n{"a":1,"tags":["x","y"]}\n3'
err json_missing.spg  "@json.get: path not found: 'b'" "has no key 'b'"
err json_invalid.spg  'invalid JSON: expected a string key' 'line 1, column'
err json_friendly.spg 'Oops! @json.get: path not found'

echo "multi-arquivo: $pass ok, $fail falhas"
[ "$fail" = 0 ]
