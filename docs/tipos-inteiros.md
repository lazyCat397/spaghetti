# Tipos inteiros: `int`, `i8`..`i64`, `u8`..`u64`

Este documento descreve o comportamento atual dos tipos inteiros.

## Tipos

| tipo | faixa | overflow sem `#> strict` | overflow com `#> strict` |
|------|-------|--------------------------|--------------------------|
| `int` (= `i64`) | -9223372036854775808..=9223372036854775807 | wrap em 64 bits | erro |
| `i8` | -128..=127 | wrap para a faixa | erro |
| `i16` | -32768..=32767 | wrap para a faixa | erro |
| `i32` | -2147483648..=2147483647 | wrap para a faixa | erro |
| `u8` | 0..=255 | wrap para a faixa | erro |
| `u16` | 0..=65535 | wrap para a faixa | erro |
| `u32` | 0..=4294967295 | wrap para a faixa | erro |
| `u64` | 0..=18446744073709551615 | wrap em 2^64 | erro |

`i64` e `int` são o mesmo tipo, com dois nomes. `f64` e `float` também. `type(x)` imprime `int` nos dois casos.

## Regras

- Tipos diferentes não se misturam. `i8 + int`, `u8 + i8` e `u64 + int` são erros de tipo, sem conversão implícita.
- Um literal ao lado de um tipo sized assume esse tipo, como no Rust: `x + 120` com `x: i8` funciona, e `120 + x` também.
- Um literal anotado é checado na faixa em compilação: `const X: i8 = 300;` é erro, no literal.
- Aritmética exige dois operandos do mesmo tipo sized, e o resultado tem esse tipo.
- Comparações (`<`, `>`, `<=`, `>=`, `==`, `!=`) funcionam entre o mesmo tipo sized. Comparações de `u64` são sem sinal.
- Variável `int` nunca se adapta a um tipo sized.

## `u64`

`u64` guarda seus 64 bits no mesmo valor que `int`. A interpretação vem do tipo estático: um `u64` com bits negativos é um número grande (`u64::MAX` tem todos os bits em 1).

- **Literais** vão até `18446744073709551615`. Um literal acima de `9223372036854775807` só vale onde um `u64` é esperado: `let x: int = 9223372036854775808;` é erro.
- **Aritmética** é sem sinal e faz wrap em 2^64 por padrão. Com `#> strict`, um resultado que sai de `u64` é erro (`addition overflow in u64`).
- **Divisão e resto** são sem sinal. Divisão por zero é erro.
- **Comparações** são sem sinal: `u64::MAX > 9223372036854775808` é verdadeiro.
- **Impressão** e `@into.string` mostram o número sem sinal: `u64::MAX` imprime `18446744073709551615`.
- **`u64::max`** é `18446744073709551615`, e `u64::min` é `0`.
- **Conversões para float:** `@into.float(x)` e `@into.f32(x)` aceitam `u64` (o valor sem sinal, arredondado uma vez).
- **Coleções:** `print` e `@into.string` de `array<u64>`, `map<K, u64>` e de opcionais mostram os números sem sinal. `array<f32>` e mapas de `f32` mostram o texto de f32 (`0.1`, não `0.10000000149011612`).
- **Structs:** cada campo guarda o shape do seu tipo: `u64` e `f32` dentro de uma struct (inclusive aninhada, ou dentro de um array de structs) imprimem certo. Enums não têm dados, então não precisam disso.

## Conversões `@into`

Para cada tipo sized há duas funções. Entrada `int`:

| função | valor fora da faixa |
|--------|---------------------|
| `@into.i8(v)`, `@into.u16(v)`, ... | erro, que aponta a versão `w` |
| `@into.i8w(v)`, `@into.u16w(v)`, ... | wrap para a faixa (`@into.u8w(300)` dá `44`) |

`@into.u64(v)` aceita `int` não negativo e erra com negativo. `@into.u64w(v)` é o padrão de bits de qualquer `int`: `@into.u64w(-1)` dá `18446744073709551615`.

Nas versões `w`, a entrada `u64` usa os bits direto: `@into.u8w(18446744073709551615)` dá `255`, porque 256 divide 2^64. Só as versões sem `w` passam o `u64` por `int`, e aí um valor acima de `i64::MAX` é erro.

Com literal, a checagem é em compilação: `@into.i8(300)` e `@into.u8(-1)` são erro de tipo, no literal, com a dica da versão `w`. As versões `w` aceitam qualquer literal. Com variável, a checagem é em tempo de execução.

Entrada de `@into.T` pode ser `int` ou qualquer tipo sized: `@into.u8w(x)` com `x: i8` e `x = -5` dá `251`.

No sentido contrário, `@into.int(x)` devolve um tipo sized como `int`. Para `u64` acima de `9223372036854775807` isso é **erro**, não wrap: o valor não cabe em `int`. `@into.u64(x)` com `x: u64` é a identidade.

## JIT e VM

O JIT é automático: funções e laços quentes viram código nativo sem nada escrito no fonte. Quando uma operação sai do caminho nativo (um wrap, um carry de `u64`, um divisor zero), a VM refaz a conta e aplica a regra exata. VM e JIT dão sempre o mesmo resultado.

## `f32`

`f32` é ponto flutuante de precisão simples. A VM guarda todo float como `f64`, então um `f32` é um `f64` cujo valor é exatamente representável em f32. Cada operação aritmética termina com um arredondamento para f32 (`OpCode::RoundF32`), e no JIT o arredondamento é um `fdemote`/`fpromote`.

- Literal `f32` é arredondado uma vez, na compilação: `let x: f32 = 0.1;` guarda `0.1` em f32. Um literal além de `f32::MAX` é erro.
- `+ - * /` e `%` seguem IEEE: o f64 intermediário não muda o resultado de f32 nesses operadores. Potência (`**`) usa `pow` em f64 e depois arredonda: pode diferir do f32 nativo em casos raros.
- Impressão e `@into.string` de `f32` usam o menor texto que volta para o mesmo f32: `0.1` imprime `0.1`.
- `@into.float(x)` leva `f32` a `float` sem mudar o valor. `@into.f32(x)` arredonda um `float` para f32. `@into.int(x)` trunca.
- Mistura `f32` com `float` é erro de tipo. Um literal decimal ao lado de um `f32` assume f32.
- Funções `@math` aceitam `f32`: o argumento entra como `float` (sem perda), a função roda em f64, e um resultado `float` volta a `f32` com um único arredondamento. Funções que devolvem `int` (`floor`, `ceil`, `round`) continuam devolvendo `int`.

## Propriedades dos tipos

Cada tipo sized tem `max`, `min` e `bits`: `i8::max` é `127`, `u16::min` é `0`, `i32::bits` é `32`. Eles têm o tipo do próprio valor (`u8::max + 1` dá `0`, com wrap).

`u64::bits` é `64`. `f32` tem `max`, `min` (o mais negativo), `epsilon`, `inf` e `nan`, todos já em precisão simples. `i64::max` e `f64::epsilon` valem por serem alias de `int` e `float`.

## Onde está o código

- regra da faixa e do wrap: `crates/spg-vm/src/arith.rs` (`sized`, `wrap_into`, `into_sized`, `u64_arith`)
- opcodes: `OpCode::SizedArith`, `OpCode::U64Arith`, `OpCode::LtU64` e irmãos, `OpCode::RoundF32` em `crates/spg-vm/src/chunk.rs`
- JIT: `crates/spg-jit/src/lib.rs` (só o caminho nativo; o resto falha a chamada e a VM decide)
- faixas e tipos: `Ty::sized_bounds` em `crates/spg-typecheck/src/typed_ast.rs`
- testes: `tests/ok/sized_*.spg`, `tests/ok/u64_*.spg`, `tests/ok/into_*.spg`, `tests/ok/f32_*.spg`, `tests/err/*_big*.spg`, `tests/err/u64_*.spg`
