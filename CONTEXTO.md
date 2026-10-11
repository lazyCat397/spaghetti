# Spaghetti (.spg) — contexto de continuidade

Linguagem nova em Rust, compilada pra bytecode + VM própria, tipagem estática.
Crates: spg-lexer → spg-parser → **spg-modules** → spg-typecheck → spg-compiler → spg-vm, mais spg-cli.
`spg-modules` resolve o grafo de imports e devolve UM `Program` fundido; do typecheck em diante
nada sabe que existiam vários arquivos.

> **Versão atual: 1.20.0** (spg e extensão alinhados; **1.20.0 = o primeiro grande update**: stack trace nos erros de runtime, interpolação `"${expr}"`, `spg build`, tipo função/lambdas/callbacks, LSP completo (assinatura, hover, definição, parâmetros, formatação), `spg fmt` e tipo opcional `T?`/`none`/`??` (seção "Release 1.20.0" no fim deste arquivo); 0.19.0 = `readLine(prompt)` (sobrecarga: escreve o prompt sem quebra de linha) + fim de entrada vira erro capturável em `readLine()` (antes devolvia `""`), igual ao `@sys.readLine` (seção \"readLine\" no fim deste arquivo); 0.18.0 = todas as funções de `@str`, `@array`, `@map`, `@math` e `@into` chamáveis como método (`x.fn(...)`), inclusive em int e float (seção \"Métodos em todos os namespaces\" no fim deste arquivo); 0.17.4 = atalhos `x.string()`, `x.int()`, `x.float()` para `@into.*` (qualquer valor; sem argumentos); o parser aceita essas palavras de tipo depois de `.`; `function string()` em struct continua impossível (palavra reservada). 0.17.3 = `@into.string(x)` aceita QUALQUER tipo (array, map, struct, enum...), com o mesmo texto de `print` (builtin `AnyToString`); int/float/bool seguem com o overload próprio. 0.17.2 = `let`/`const` com valor void vira erro de tipo (VoidValue) em vez de "VM bug: operand stack underflow"; ex.: `let x = if (c) { print(..) } else { print(..) }`. 0.17.1 = release de teste do `update-spg.bat`; 0.17.0 = `@map.keysOf(m, valor)` (seção "`@map.keysOf`" no fim deste arquivo); 0.16.0 = namespace `@json` com 9 funções (seção "`@json`" no fim deste arquivo); 0.15.0 = diretivas de arquivo `#>` (`friendly-terminal`, `timeout N`, `strict`) com LSP e extensão cientes delas (seções "Diretivas de arquivo" no fim deste arquivo); 0.14.0 = `@file` completo: deleteAll, size, modified, bytes; 0.13.0 = `@file` e `@sys`; 0.12.0 = `throw` + `try`/`catch` de verdade; 0.11.0 = métodos nativos `s.len()` / `xs.push(1)` / `m.has(k)`; 0.10.0 = `@str` completo, `@array.pop` vazio vira erro, literal de array homogêneo). 0.8.1 = autocomplete de `p.`/`Struct { ` (só LSP); 0.9.0 = `@array` e `@math` completos (seção "Sessão `@array`/`@math` completos"). 0.6.2 = só ícones novos; 0.7.0 = `map<K,V>` (seção "Sessão `map`"); 0.8.0 = enum com valores + `array == array` + `[]` vazio (seção "Sessão `enum` com valores"). A linguagem voltou a crescer depois de "fechada":
> veja "Sessão de expansão da linguagem (2026-10-04)" (`'...'`, `a..b`, métodos de struct) e "Sessão `enum` (2026-10-05)" (enum percorrível).
> Suíte `examples/multi/run.sh`: **290 ok**; LSP 40 testes. `map<K,V>` (0.7.0), `enum` com valores (0.8.0), autocomplete de structs (0.8.1) `@array`/`@math` completos (0.9.0) e `@str` completo (0.10.0) FEITOS. Próximo: ver "Plano da PRÓXIMA sessão" (lacunas do pacote).

## Como preparar o ambiente (sessão nova)

    bash setup-rust.sh          # apt-get puro; rustup.rs costuma estar bloqueado
    # (a raiz do projeto — onde está o Cargo.toml — é a própria pasta spaghetti-prot)
    cargo build --release
    ./target/release/spg examples/hello.spg

## O que já está pronto (v2.0 candidata)

- ~40 builtins em camelCase via `OpCode::CallBuiltin(BuiltinId, argc)` genérico, agora
  organizados em **namespaces com `@`** (`@str @array @into @math @time`, seção abaixo). Tabela
  com overloads em `spg-typecheck/src/builtins.rs` (chaves `"@math.sqrt"`, `"@time.now"`...), execução em
  `spg-vm/src/builtins.rs`.
- Indexação de array: `arr[i]`, `arr[i] = v`, `arr[i] += v`, aninhada (`m[i][j]`).
- Structs completos: `struct P { x: int }`, `P { x: 1 }`, `p.x`, `p.x = v`,
  `p.x += v`, aninhamento total. Literal com `{}` (colchetes ficam pra listas).
- Suíte em `examples/` (rodar tudo em release após qualquer mudança).
- **Globais reais**: todo `let`/`const` no escopo raiz do arquivo é um slot global
  (`OpCode::GetGlobal/SetGlobal`, `Vm.globals`), visível e mutável dentro de funções.
  Locais fazem shadowing. Só enxerga globais declarados ANTES da função (regra do typecheck).
- **Multi-arquivo completo**: `import { a } from "./x.spg"`, `export` antes de
  function/let/const/struct, e **re-export** `export { a } from "./y.spg"`.

## Multi-arquivo — como funciona (CONCLUÍDO)

Decisões do usuário, todas implementadas:
1. Só import nomeado `{ a, b }`. Sem `import * as`, sem default.
2. Ciclo de import = erro fatal, mostrando a cadeia inteira (`a -> b -> a`).
3. Caminho SEMPRE com `.spg` explícito.
4. `import { x }` só traz `x` pro escopo LOCAL; só vira importável de fora com
   `export { x } from "..."`. Importar não re-exporta (o erro sugere o `export` certo).
5. Um erro semântico = um diagnóstico (nunca deduplicar no final, corrigir a origem).
6. Nome mangled só existe por dentro; erros mostram o nome original.

Design (`crates/spg-modules`): cada símbolo top-level de módulo NÃO-entrada vira
`<stem>#<hash>::<nome>`; o arquivo de entrada NÃO é renomeado (arquivo único se comporta
idêntico). `rename.rs` reescreve declarações e usos com rastreio de shadowing (parâmetro,
`let` em bloco, var de for-in/catch shadowam símbolo de módulo). Módulo carregado uma vez
(diamante). `ModuleInfo` separa `symbols` (declarados), `reexports` (importáveis de fora mas
NÃO nomes dentro da fachada) e `imported`. O símbolo re-exportado mantém o mangled da ORIGEM,
então `a.x` e `fachada.x` são o mesmo símbolo/estado.
`MANGLE_SEP` e `display_name` moram em `spg-parser/src/ast.rs` (única dependência comum).

Testes: `bash examples/multi/run.sh` (a partir da raiz do projeto) = 122 checks (eram 43 aqui) em 3
seções: `ok` (saída exata), `err` (mensagem + linha do erro e da nota), `diag` (contagem
exata de erros + proíbe vazamento de `Named(`, `Array(`, `x#hash::`). Toda mudança nele deve
ser provada por SABOTAGEM (quebrar o código de propósito e ver o check falhar).

## Erros de tipo mostram o arquivo (CONCLUÍDO — fecha o pacote .spg por completo)

`Pos` ganhou `file: Option<String>`. O parser sempre gera `None` (só vê tokens); quem carimba
é `spg-modules/src/annotate.rs`, uma varredura do AST inteiro rodada logo após o parse de
CADA módulo (entrada incluída), antes do merge. `error.rs` do typecheck ganhou `pos()` +
`location()` centrais em vez de tocar as 25 variantes de `TypeError` uma a uma — toda
mensagem agora é `"type error in <arquivo> at line X, column Y: ..."`. Resolve o caso que
motivava isso: dois arquivos diferentes com erro na mesma `line, column` eram indistinguíveis.

## Pendências conhecidas (fora do escopo até aqui)

- Sem `%=` por decisão de design.

## CLI + LSP + extensão VSCode (CONCLUÍDO nesta sessão)

- **Zero dependências externas**: `thiserror` removido (Display/Error manuais em `LexError`
  e `ParseError`); o Cargo.lock só lista os crates do Spaghetti. `[profile.release] strip=true`
  (14 MB → 1,2 MB, sem mudar código gerado). `.cargo/config.toml` liga o CRT estático no alvo
  `x86_64-pc-windows-msvc`.
- **CLI `spg`** (`crates/spg-cli`): `spg f.spg` | `run` | `check` | `lsp` | `--version` |
  `--help`; flags `--time`/`--dump-bytecode`; saída 0/1/2. Hook de panic: `Broken pipe`
  sai quieto (`spg x | head`), outros panics viram "internal error".
- **`spg-lsp`** (crate novo, só std): JSON e JSON-RPC escritos à mão; `uri.rs` (URIs do
  Windows `file:///c%3A/...`, testadas em qualquer SO), `diag.rs` (resolve→typecheck→compile em
  `Diag{file,line,col,message}` + `utf16_range`), `server.rs` (didOpen/Change/Save/Close,
  publishDiagnostics, completion com keywords/tipos/builtins com assinatura/símbolos do
  arquivo). Reanalisa TODOS os documentos abertos a cada mudança (um import quebra outro).
  Erro localizado em OUTRO arquivo aparece na linha 1 do arquivo aberto com "in <arq>:L:C: ...".
- **`spg-modules`**: `resolve_with(entry, overlay)` (buffers não salvos), `canon()` (tira o
  prefixo `\\?\` do Windows), `ModuleError::locate()`. **`spg-typecheck`**: `TypeError::message()`.
- **BUG DO LEXER CORRIGIDO**: lia bytes com `b as char` → `"olá"` virava `olÃ¡` e colunas
  contavam bytes. Agora `Vec<char>` (coluna = caractere). Comparado com o motor original:
  18/18 exemplos idênticos. (Na época a VM ainda contava bytes; HOJE NÃO: `len("olá")` = 3 e todo
  `@str.*` trabalha em caracteres, com teste em `ok/str_chars.spg`.)
- **Extensão** (`editors/vscode`, `dist/spaghetti-0.2.0.vsix`): sem node_modules, cliente LSP
  manual em `extension.js`, gramática TextMate, snippets, ▶ Run (Ctrl+F5) e Check via tasks,
  reinício automático do servidor (máx 3 em 60 s), ícones do próprio usuário (garfo girando espaguete; `icon.png` 720x720 e `file-icon.svg` com `viewBox` adicionado); a task de Run usa `clear:false` (o terminal acumula a saída entre execuções, pedido do usuário). Empacotada com o `@vscode/vsce`
  oficial (instalado só como ferramenta, fora do entregável).
- **Windows**: `build.bat` → `install.ps1` (copia p/ `C:\spaghetti`, PATH do usuário via registro,
  `code --install-extension`) e `uninstall.ps1`.
- **Testes**: 23 unitários no spg-lsp + 24 e2e (cliente Python × binário real) + 9 cenários da
  extensão com `vscode` mockado (Node) + 30 escopos da gramática (vscode-textmate). Todos
  provados por sabotagem. Suíte multi-arquivo: 55/55.
- **NÃO testado (sem Windows/PowerShell/VSCode no container)**: `build.bat`, `install.ps1`,
  `uninstall.ps1`, a instalação real do .vsix no VSCode e o `spg.exe` em si. Se algo falhar
  no Windows, começar por aí.

## Sessão "extensão + instalador" (2026-10-02) — CONCLUÍDA

> ATENÇÃO: os detalhes de namespace desta seção (`math.` sem `@`, `string` como tipo E namespace,
> `sortText: "zz_string"`) foram SUPERADOS pela sessão `@namespaces` mais abaixo. O resto vale.

**Completion de namespace (LSP + extensão 0.2.0):**
- A extensão registrava o provider SEM trigger character: o `.` nunca abria a lista sozinho.
  Agora `registerCompletionItemProvider(LANG, provider, '.')`.
- `server.rs`: `dot_context()` devolve `Namespace(ns) | Other | None`. Depois de `ns.` só os membros;
  depois de QUALQUER outro `.` (campo de struct, `1.`, `f(x).`, ponto dentro de string) a lista vem
  VAZIA (antes despejava a lista global, o que agora, com o `.` como gatilho, seria ruído constante).
- Escolher um namespace na lista insere `math.` (campo `insertText`) e dispara
  `editor.action.triggerSuggest` (campo `command`) -> os membros abrem sozinhos. A extensão repassa
  `insertText/sortText/filterText/command` em `toCompletionItem`.
- `string` é tipo E namespace: os dois aparecem; em posição de tipo (`:`, `->`, `<`) o tipo vem
  primeiro, em expressão o namespace (via `sortText: "zz_string"` no que não serve).
- Testes: 4 novos no spg-lsp (27 no total) + `tools/ext-tests/` (6 cenários com `vscode` falso contra
  o `spg lsp` REAL). Todos provados por sabotagem.

**Instalador interativo (`build.bat` -> `install.ps1`):** `build.bat` virou só lançador (repassa
args, `pause` só se aberto por duplo clique, propaga o código de saída). `install.ps1` faz 5 etapas:
checagens (cargo, VSCode, extensão instalada, build anterior) -> perguntas + plano -> build ->
instalação -> verificação (`--version` + self-test que roda um programa de verdade).
- Pasta de instalação: vazia = usa; install Spaghetti = oferece atualizar (mostra versão antiga ->
  nova); sobras só nossas (`examples`, `spg.exe.old`) = reaproveita; **qualquer outra coisa (pasta
  alheia ou ARQUIVO com esse nome) = pergunta: substituir (exige digitar `YES` maiúsculo) ou renomear
  (sugere `nome-old`, rejeita nome que já existe/inválido, `cancel` aborta)**. `-Yes` NUNCA toca em
  pasta que não reconhece (erro).
- Nada fora da pasta do fonte muda antes do build dar certo (build falhou = rename/delete não
  acontece). Recusa pastas perigosas (raiz do drive, perfil, Program Files, a própria pasta do
  fonte, pai dela). Se o `spg.exe` está em uso (servidor LSP do VSCode): pergunta se encerra, senão
  move o antigo pra `spg.exe.old`. PATH do usuário via registro cru (preserva `%VAR%`, sem duplicar).
- Perguntas: pasta, atualizar, PATH, extensão (mostra versão atual -> nova), copiar exemplos,
  self-test, apagar `target\` depois (padrão NÃO), "Go ahead?". `-Yes -NoBuild -InstallDir -Exe -Vsix`.
- `uninstall.ps1`: mostra o que achou, pergunta, para processos spg, tira PATH/SPG_HOME/exe/exemplos/
  extensão; arquivos do usuário na pasta NUNCA são apagados (a pasta só some se ficou vazia).
- **TESTADO agora com PowerShell 7 real no container** (`tools/installer-tests/run.ps1`, 115 checks,
  `pwsh -File tools/installer-tests/run.ps1 -SpgBin target/release/spg [-Only nome] [-Show]`):
  registro, comando `code`, teclado e cargo são fakes; pastas, rename/replace, cópia, `--version` e
  self-test rodam de verdade (com o binário spg do Linux). 14 sabotagens, todas pegas. Achou 3 bugs
  reais no instalador antes de entregar (abaixo). **Continua NÃO testado no Windows de verdade**:
  registro real, `code` real, `Start-Process`, `cargo` real no PowerShell, `Get-Process ... .Path`.
- Bugs pegos ao testar (não repetir): `Split-Path -LiteralPath x -Parent/-Leaf` é combinação INVÁLIDA
  (usar `[IO.Path]::GetDirectoryName/GetFileName`); em string, `"$var?"` no PowerShell 7 lê o `?`
  como parte do nome (usar `${var}?`); `$home` é variável automática somente-leitura (não atribuir).
- Armadilha de teste: uma sabotagem ("apagar cache sempre") apagou o `target/` REAL do projeto porque
  os cenários usavam a raiz real; agora cada cenário roda numa raiz-fake (`New-FakeRoot`) e o harness
  envolve cada cenário em try/catch (exceção = FAIL, antes o script morria calado e a sabotagem
  parecia "não pega").
- Pendente do instalador: nada obrigatório. Ideias: `install.sh` p/ Linux/macOS, instalar `spg` por
  usuário em `%LOCALAPPDATA%` como alternativa a `C:\spaghetti`.

## Namespaces de builtins com `@` (sessão 2026-10-02 — CONCLUÍDO)

Decisão do usuário: nada de builtin solto. Só sobram globais `print`, `printError`,
`readLine` e **`len`** (único atalho, de string e array; "só parece o certo", sem mais motivo).
Todo namespace se escreve com `@`: `@str`, `@array`, `@into`, `@math`, `@time`.

| Namespace | Membros |
|---|---|
| `@str` | `len toUpper toLower trim split substring charAt contains startsWith endsWith replace indexOf repeat` (era `string.`) |
| `@array` | `len push pop contains indexOf slice reverse sort join` (era `list.`) |
| `@into` | SÓ três: `string` (int/float/bool), `int` (float ou string), `float` (int ou string) (era `transform.`) |
| `@math` | `abs floor ceil round sqrt pow min max random` + `pi(n?)` e `e(n?)` |
| `@time` | `now() -> string`, `unix() -> int`, `elapsed() -> string` (era `run`, que era float), `sleep(ms: int) -> void` |

- **`@` no lexer** (`TokenKind::Namespace(nome_sem_arroba)`, `Lexer::read_namespace`): `@foo` com `foo`
  fora de `spg_lexer::NAMESPACES` é `LexError::UnknownNamespace` (lista os 5 válidos); `@` solto
  também. Como `@` nunca faz parte de identificador, **não há mais colisão**: `time`, `math`, `list`,
  `transform` e até `str` são nomes comuns de função/let/param/struct/campo (teste
  `ok/ns_nomes_livres.spg`). `string` continua só tipo. Namespace em posição de NOME (`let @math`,
  `struct @str`, param, campo, tipo) = `ParseError::ReservedName` ("'@math' is a reserved namespace
  and cannot be used as a name"), via `parse_ident_name`. `@math` sem `.membro` = `NamespaceWithoutMember`.
- **Como funciona (sem opcode novo, sem valor "namespace" em runtime)**: `parse_namespace_access`
  gera UM `Expr::Ident("@ns.membro")`; o postfix de chamada vira `Call` normal e o typecheck procura
  essa chave na tabela de builtins. O nome do membro passa por `parse_member_name`, que aceita as
  keywords de tipo (`int`, `float`, `string`...) — sem isso `@into.int` não parseia.
- **`@into`**: `@into.int(42.0)`=42 (trunca pra zero: `5.9`→5, `-5.9`→-5), `@into.int("42")`=42 (aceita
  espaços nas pontas), `@into.int("abc")`/`""` = `InvalidNumberFormat` (erro de execução, exit 1),
  `@into.float(3)`=3.0, `@into.float("3.5")`=3.5. `parseInt`/`parseFloat`/`toInt`/`toFloat`/`toString`
  sumiram (os `BuiltinId` internos `ParseInt` etc. continuam: o typecheck escolhe pela overload).
- **`@math.pi(n)` / `@math.e(n)`**: `n` = CASAS DECIMAIS, com ARREDONDAMENTO (`pi(4)` = 3.1416, não
  truncado 3.1415). Sem argumento = 2 casas (3.14 / 2.72). `n > 15` é limitado a 15 (f64 não tem mais
  que isso; `pi(40)` = `pi(15)` = 3.141592653589793). `n < 0` = `RuntimeError::NegativeDecimals`.
  Arredonda via formatação decimal (`format!("{:.*}")` + parse), não `x*10^n` (evita erro do float).
  Argumento é `int`: `pi(1.5)` é erro de tipo. `BuiltinId::{PiDecimals, EDecimals, PiDefault, EDefault}`.
- **`len` e `@str.*` em CARACTERES** (não bytes): `len("olá")` = 3, `@str.indexOf("açúcar","car")` = 3,
  `charAt`/`substring`/`split("")` por caractere. A VM já fazia isso (o aviso antigo de "len conta
  bytes" estava desatualizado); agora tem `ok/str_chars.spg` provado por sabotagem (len→bytes e
  indexOf→bytes derrubam o teste).
- **Migração com dica de erro** (sem aliases, quebra total):
  - nomes soltos antigos → `RemovedBuiltin` (`'sqrt' is no longer a global function; use '@math.sqrt'`;
    `contains`/`indexOf` apontam pros dois; `clock` → `@time.unix()`/`@time.elapsed()`);
  - grafia antiga `math.sqrt(...)`, `list.x`, `transform.toInt`, `time.run` → `TypeError::OldNamespace`
    com a substituição certa (`renamed_namespace_call`, só sugere nome que existe na tabela); só
    dispara se `math`/`time`/`list`/`transform` NÃO é variável declarada (agora são nomes livres);
  - `string.len(...)` não chega ao typecheck (`string` é keyword): `ParseError::OldStringNamespace`
    ("use '@str.len'").
  - **Bug antigo achado e corrigido**: chamar algo que não é nome (`p.x()`, `f()()`) era IGNORADO em
    silêncio (o checker dava `return None` sem erro; o programa rodava e saía 0 sem executar a
    chamada). Agora `TypeError::NotCallable` ("only named functions can be called"). Sem isso a grafia
    antiga `math.sqrt(x)` também seria engolida em silêncio.
- `@math.foo` inexistente: `namespace '@math' has no member 'foo'`.
- **`@time.elapsed()` devolve TEXTO já formatado** (pedido dele, 2026-10-02: "como o elapsed do Rust, que se
  adapta"): tempo desde o início do programa (relógio MONOTÔNICO; `Vm::new` chama
  `mark_program_start()`, `OnceLock` global, por PROGRAMA e não por arquivo), em inteiros e unidades
  `h m s ms µs ns`: a maior unidade que cabe + a seguinte SÓ se não for zero (`45ms`, `3s 250ms`,
  `1h 2m`; 2h 3m 4s vira `2h 3m`; `1h 0m 5s` vira `1h`). Sem `ps` (o relógio é em ns, seria sempre 0) e
  sem `d` (1 dia = `24h`). `µ` é U+00B5 (o mesmo do `Duration` do Rust). Função `format_duration` em
  `spg-vm/src/builtins.rs`, com 2 testes unitários. **Consequência: não dá mais pra SUBTRAIR dois
  `elapsed`; pra medir um trecho usa-se a diferença de dois `@time.unix()` (int, ms inteiros).** Não
  existe medidor de trecho em ns ainda (ideia aberta, não pedida).
  `@time.unix()`: `int` ms desde 1970. `@time.now()`: ISO-8601 UTC (`2026-09-30T14:03:22Z`, algoritmo
  Hinnant, sem crates). `@time.sleep(ms)`: bloqueia; negativo = `NegativeSleep`; `sleep(0)` ok.
- **LSP/extensão**: `triggerCharacters: [".", "@"]`. Depois de `@`/`@par` só os 5 namespaces; escolher
  um troca `@par` por `@math.` via `textEdit` (o editor NÃO trata `@` como parte da palavra, então sem
  `textEdit` viraria `@@math.`) e reabre a lista; depois de `@math.` só os membros; `math.` sem `@`
  e qualquer outro `.` = lista vazia. A extensão repassa `textEdit.range` como `item.range`.
  `language-configuration.json`: `wordPattern` aceita `@` na frente. Gramática TextMate:
  `@(str|array|into|math|time)` = `entity.name.namespace.spg`, membro = `support.function.builtin.spg`.
- **Testes** (todos provados por sabotagem): `ok/ns_into`, `ok/ns_pi_e`, `ok/str_chars`,
  `ok/ns_nomes_livres`, `ok/ns_todos|ns_lib|ns_modulo|ns_campos` (o `ns_todos` prova o ponto zero do `elapsed`: no começo NÃO pode conter `h`, senão é o bug antigo do clock); `err/` com `ns_reservado_*` (let, fn,
  param, struct), `ns_desconhecido`, `ns_arroba_solto`, `ns_*_antigo`, `chamada_campo`, `into_*`,
  `pi_*`/`e_negativo`; 2 unitários novos no lexer (4 no total); 27 no spg-lsp; 2 no spg-vm (`format_duration`); 7 cenários em `tools/ext-tests`.
  Sabotagens que o teste PEGOU: pi truncando, pi sem clamp, len/indexOf em bytes, parse inválido→0,
  `NotCallable` mudo, `elapsed` contando desde 1970, `format_duration` sempre mostrando a 2ª unidade, namespace aceito como nome, lexer aceitando `@qualquer`, LSP tratando `math.`
  sem `@` como namespace, LSP sem `textEdit`, extensão sem `range`/sem trigger `@`.
- **Time-out no meio de uma sabotagem deixa o código SABOTADO**: aconteceu (parser aceitando
  `@math` como nome) e um "check de presença por trecho" não percebeu porque o mesmo trecho existia
  noutro lugar. Prevenção: tirar um snapshot limpo de `crates/` antes e conferir `diff -r` depois de
  cada sabotagem; rodar no máximo 2 sabotagens (≈40 s de build cada) por chamada.

## Performance e otimizador (sessão de velocidade, 2026-10-01)

Resultado: **~3,6x mais rápido** no conjunto de benchmarks (`tools/perf/bench`), de 2x (sieve) a
6x (collatz/loop_arith). Baseline antigo: fib(35) ~4s. Saída idêntica, 0 divergências em ~2000
programas aleatórios.

**Ferramentas (usar SEMPRE antes de mexer em velocidade/otimizador):**

    python3 tools/perf/bench.py base=./spg_antigo,new=target/release/spg 3   # alterna binários
    python3 tools/perf/fuzz/run.py target/release/spg 0 500                  # fuzz diferencial
    spg --no-opt prog.spg      # sem superinstructions = oráculo/referência

`examples/multi/run.sh` também compara TODO .spg do projeto com e sem `--no-opt` (seção `same`).

**O que foi feito (em ordem de impacto):**
1. `Vm::step` virou `#[inline(always)]` dentro do laço de `run_chunk`. Antes era uma função
   separada com prólogo de 6 pushes + frame de 0x158 bytes executado A CADA instrução (~14 ciclos
   por opcode). Sozinho: +50%. Efeito colateral (frame de `run_chunk` maior) compensado rodando o
   CLI inteiro numa thread com stack de 1 GB (`VM_STACK_BYTES`): recursão profunda foi de ~12 mil
   para >100 mil níveis (e no Windows a stack principal é só 1 MB).
2. Superinstructions (`spg-compiler/src/peephole.rs`, cabeçalho do arquivo lista todas): store
   (`SetLocal;Pop`), aritmética/compare local-com-constante, `IncLocal`, compare+`JumpIfFalse` em 4
   formas, `IndexGetLocals`, `FieldGetLocal`, `op+store`, `GetLocal2`, `AddLocalsStore`... Um corpo
   de loop foi de ~24 para ~9 instruções.
3. Inversão de loop: o `Jump` de volta ao topo vira o compare NEGADO apontando pro corpo (sem
   opcode novo).
4. Pool de `Vec` de locals por chamada (`Vm::pool`): chamadas +10-30%.

**BUG REAL corrigido (existia no build anterior):** o peephole fundia instruções que eram ALVO de
salto (`(if (c) { a } else { b }) < d` dava resultado errado e vazava stack). Regra agora: só a
PRIMEIRA instrução de uma fusão pode ser alvo de salto (`limit` em `match_at`). Teste:
`examples/multi/ok/opt_jump_targets.spg`.

**Medido e DESCARTADO (não repetir):**
- Tirar `Result` do caminho quente (`pop`/`expect_*` com panic frio): 0% (LLVM já otimizava).
- `#[inline(always)]` só nos helpers `binary_int` etc.: 0%.
- Uma alocação por chamada (capacidade `num_locals` direto): 0% (já era 1 malloc).
- Locals na stack compartilhada (args viram locals in-place): chamadas +30% mas loops -30%
  (acesso `stack[base+slot]` é mais caro que `Vec` dentro do frame). Variante com arena de locals
  separada: pior em tudo (0,76x). Acessar locals por referência indireta é o problema.
- Fusão `GetLocal; JumpIfFalse` (`JumpIfFalseLocal`): consistentemente mais lenta; removida.

**Cuidados de medição:** ruído de ±10-20% por janela; efeitos < 10% só valem com várias rodadas e
o agregado da suíte. Fusão nova => medir "com vs sem" no mesmo binário, e provar o fuzzer por
SABOTAGEM (quebrar a fusão de propósito e ver o fuzzer acusar; assim se achou que o gerador não
exercitava loops com limite em variável).

**Ideias abertas (não feitas):** stack de operandos de tamanho fixo + `sp` em registrador (exige
`unsafe` ou reestruturação; ataca o custo de push/pop do `Vec`); `OpCode` ainda tem 32 bytes
(`u32` nos slots encolheria); `FieldSet`/`IndexSet` compostos (`s.x += v`, `a[i] += v`) e
`IndexSet` com valor constante; erro limpo de "stack overflow" em vez de abortar o processo
(profundidade >~1 milhão); `i64::MIN / -1` ainda dá panic na VM.

## Sessão de velocidade 2 (2026-10-03) — perfil de release, builtins e print

Ideias de refatoração do `vm.rs` avaliadas e NÃO feitas (só organização, zero ganho): (1) `macro_rules!` p/ as
variantes aritméticas — o `step()` já usa helpers `#[inline(always)]` (`binary_int`...), código gerado idêntico;
(2) `impl Value { fn add }` — dispatch genérico em runtime por `match (a, b)`, vai CONTRA a tipagem estática
(opcodes tipados decidem o tipo no compile-time) e tende a ficar mais lento. Biblioteca externa NÃO ajuda no
dispatch (jump table do `match` já é o mínimo; threaded code/`become` é nightly; mimalloc/smallvec quebram a
regra de zero dependências).

**Adotado:**
- `lto = "fat"` no `[profile.release]` do Cargo.toml: **1,07x** no `tools/perf/bench` (7 rodadas; 8 de 11 melhores,
  piores: structs 0,93x, sieve/collatz ~0,97x). Build do `spg` ~44 s (era ~40 s).
- **[SUPERADO pela Sessão 5 abaixo: hoje o buffer é global, também no terminal, com thread de flush]** stdout bufferizado no `print` (`spg-vm/src/builtins.rs`: `OutMode`, `print_line`, `flush_stdout`): quando stdout
  NÃO é terminal (pipe/arquivo) usa `BufWriter` de 64 KB; em terminal mantém linha a linha (nada atrasa no ▶ Run).
  Modo decidido no 1º `print`, por thread (a VM roda numa thread só). Flush antes de `readLine`, `printError`,
  `@time.sleep`, no fim de `Vm::run` (ok ou erro, ANTES do CLI imprimir "runtime error") e no hook de panic do CLI.
  Quebra de pipe: `panic!("failed printing to stdout: ...")` com a MESMA mensagem do `println!`, o hook já sai quieto
  em "Broken pipe" (se tudo cabe no buffer e o leitor fechou antes, sai 0, sem erro). O panic acontece DEPOIS de soltar o
  `borrow_mut` (o hook chama `flush_stdout`, que usa `try_borrow_mut`; sem isso seria panic duplo = abort).
  1M de `print` p/ pipe: **1055 ms -> 157 ms** (6,7x); p/ `/dev/null`: 335 -> 157 ms; saída idêntica (md5).
  Testes novos em `examples/multi/run.sh` (`ok/print_order`, `ok/print_big` 30000 linhas > buffer, `err/print_before_error`,
  helpers `exact1`/`lines`), os 3 provados por sabotagem (printError sem flush, sem flush no fim do run, descartar linhas
  com buffer cheio). NÃO coberto por teste automático: flush antes de `readLine`/`sleep` (só observável por tempo;
  verificado à mão com pty). Limitação conhecida: abort por stack overflow perde o que ainda estava no buffer (só em pipe).

**Medido e DESCARTADO (não repetir):**
- `codegen-units = 1` (sozinho ou com LTO): **0,80x**, mais lento em todos os benchmarks.
- Reaproveitar um buffer de argumentos no `CallBuiltin` (drain da stack em vez de `Vec::with_capacity` + `reverse`):
  0% (117 ms vs 110 ms em 1M de `contains`); o malloc de bloco pequeno não era o gargalo.

**Diagnóstico de custo por operação (1M de iterações de laço, ~36 ms só o laço):** builtin trivial (`@array.len`) +38 ns;
`@array.contains` em 4 itens +25 ns além disso (posição do item quase não importa); `names[3] == "dani"` +52 ns.
O custo fixo de ~38 ns por builtin ainda não foi explicado (hipótese NÃO verificada: `call_builtin` é um `match`
gigante com prólogo/frame grande, como o `step` era antes do `#[inline(always)]`). Sem `perf` no container.
`len` de string é `chars().count()` (O(n)).

**Próximos candidatos de velocidade:** `OpCode` 32 -> 16 bytes (slots `u32`), `IndexSet`/`FieldSet` compostos, fast path
inline dos builtins quentes (`len`, `contains`, `indexOf`) pra fugir do `match` gigante.

## Sessão de velocidade 3 (2026-10-03) — operações de índice de array (fusões)

**Medir em LOCAIS, não em globais.** Variáveis de topo são globais (`GetGlobal`), sem nenhuma fusão `...Locals`; o código
quente de verdade fica em funções. Micro-benchmarks de índice só valem dentro de `function run() { ... }`.

Custo extra por operação num laço de 3M iterações (laço vazio ~17 ns/iter): `k += a[k]` +15 ns; `a[k] = i` +33 ns;
`a[k] = a[k] + 1` +37 ns; **`a[k] += 1` +67 ns** (o composto era 1,8x PIOR que a forma longa: `GetLocal2; Dup2; IndexGet;
AddConst; IndexSet`).

**Adotado** (tudo em `chunk.rs` + `peephole.rs` + corpos fora de linha em `spg-vm/src/index_ops.rs`, arms de 1 linha no `step`):
- `IndexGetDupLocals(arr, idx)` = `GetLocal; GetLocal; Dup2; IndexGet` (cabeça de todo `a[i] OP= v`): `a[k] += 1` 241 -> 158 ms.
- `IndexAddConstLocals{arr,idx,k}` = cabeça + `AddConst(k)` + `IndexSet` (cobre `+= c` e `-= c`): **241 -> 56 ms** (4,3x; ~67 ns -> ~1,5 ns).
- `Index{Add,Sub,Mul}LocalLocals` (int) e `Index{Add,Sub,Mul}FloatLocalLocals` (float) = cabeça + `GetLocal(x)` + op + `IndexSet`
  (`a[i] OP= x` com x local): ~190 -> ~57 ms (3,3x).
- `IndexSetLocalsLocal{arr,idx,v}` (`a[i] = v`), `IndexSetLocalsConst{arr,idx,c}` (`flags[m] = false`), `IndexSetLocalLocal{idx,v}`
  (`m[r][c] = v`, linha já na pilha): ~156 -> ~65 ms (2,4x; 2D 1,6x). **`sieve` 1,57 s -> 1,04 s (1,5x)**.
- Cada fusão exige índice e array em LOCAIS e valor local/constante; valor = expressão qualquer NÃO funde (reordenar o valor
  antes do bounds-check só seria seguro para expressões puras e infalíveis; o driver do peephole só tem padrões de tamanho fixo).
- Ordem das checagens de erro idêntica à sequência original (tipo do operando, do índice, do array, bounds, tipo do elemento);
  mensagem `index out of bounds: N (array has M element(s))` preservada.

**Medido e DESCARTADO (não repetir):**
- Caudas genéricas `<op>; IndexSet` (`IndexSetAddInt` ... + `AddConst/MulConst; IndexSet`, 8 opcodes): neutro/pior
  (`a[k] += x` 190 -> 199 ms, `+= x*y` 165 -> 181 ms). O ganho está em fundir o STATEMENT inteiro (sem tráfego de pilha/`Rc`).
- Um único `OpCode::Idx(IdxOp)` com sub-despacho (`index_ops::exec`) para manter o `step()` pequeno: mais lento que um arm
  por op (+5-10% nas próprias ops de índice, e não melhorou os laços aritméticos).
- Só mover os corpos para fora de linha NÃO muda o custo nos laços aritméticos; mantive assim porque tira código do `vm.rs`.

**Ruído de layout:** um controle SEM mudança de comportamento (só uma função inerte no `vm.rs`) já oscila +-5% em
`loop_arith`/`collatz`. Custo residual medido dos opcodes novos em laços puramente aritméticos: ~+3% no total sem `sieve`
(controle inerte +0,9%). `structs` (0,86x numa rodada) = ruído: 11 rodadas dão 1189 ms original vs 1197 ms final.
Sempre comparar com >= 9 rodadas e incluir um binário-controle quando a diferença for < 8%.

**Testes:** `ok/index_fused.spg` (valores conferidos à mão; os 11 opcodes aparecem no dump; usa índices em VARIÁVEIS locais:
literal como índice NÃO gera a fusão e esconde mutações), `err/index_fused_oob_{addconst,addlocal,setlocal,setconst,set2d,head}.spg`.
Provados por sabotagem (+k -> -k, float_add -> sub, float_mul -> add, sem bounds no `store`/`add_const`/`SetLocalsConst`/
`AddLocalLocals`/`GetDupLocals`, 2D lendo o local errado). **O fuzz tinha um buraco:** `idxvars` nunca era preenchido, então
nunca gerava `a[ix]` com índice em variável. `tools/perf/fuzz/gen.py` agora tem variáveis de índice reais (que mudam, e às
vezes saem do limite), `*=`, arrays `float`/`bool`/2D e atribuições com valor local/constante. Sozinho ele detecta a mutação
`+k -> -k` (5 divergências em 120 programas). `fuzz_progs/` é gerado na raiz: apagar antes de empacotar.

**Armadilha de processo:** `sab.sh` restaura o código-fonte mas NÃO o binário; sempre `cargo build` limpo depois de sabotar.
`cargo build` com LTO leva ~45-70 s e `cargo test` > 100 s: um comando por vez, com `timeout`.

**Ainda aberto:** ver o fim da Sessão 5.

## Sessão de velocidade 4 (2026-10-03) — builtins no caminho rápido + charAt/substring

Custo por chamada de builtin ANTES (3M chamadas, laço vazio 13 ns/iter): `@array.len` 49 ns, `@str.len` 56, `@math.abs` 54,
`@array.contains` 83, `@array.push` ~83, `@math.sqrt` 89; **`@str.charAt` ~19 us por chamada numa string de 20 mil caracteres**
(coletava `Vec<char>` inteiro a cada chamada: laço sobre a string = O(n^2)).

**Adotado:**
- `builtins::call_builtin_on_stack(stack, id, argc)` (`#[inline(never)]`): o arm `CallBuiltin` do `step` virou 1 linha. Os builtins
  quentes e simples (`ArrayLenBuiltin, Len, ArrayContains, ArrayPush, ArrayPop, Abs, AbsFloat, Min/MaxInt, Min/MaxFloat, Floor, Ceil,
  Round, Sqrt, IntToFloat, FloatToInt`) operam direto na pilha, sem `Vec` de argumentos nem o `match` gigante de `call_builtin`;
  o resto cai no caminho genérico de sempre (`drain` dos argumentos + `call_builtin`). Mesma expressão de cada arm, operandos
  desempilhados na ordem inversa. Custo por chamada: `@array.len` 49 -> 30 ns, `abs` 54 -> 27, `sqrt` 89 -> 40, `contains` 83 -> 60,
  `push+pop` 126 -> 86. Os 4 helpers `as_string/as_int/as_float/as_array` agora são do módulo (antes eram aninhados em `call_builtin`).
- Peephole 1->1: `CallBuiltin{ArrayLenBuiltin, argc:1}` -> `OpCode::ArrayLen` (opcode que já existia): `@array.len` 30 -> 23 ns.
- `CharAt`: se os bytes `[..=idx]` são ASCII o char é o byte `idx` (O(idx) vetorizado); senão `chars().nth(idx)`; o `len` do erro
  continua sendo contagem de CARACTERES. **charAt num laço de 20 mil: 388 ms -> 13 ms (28x).**
- `Substring`: `is_ascii()` + fatiamento por bytes; não-ASCII usa `skip/take` (sem `Vec<char>`). 66 -> 40 ms (200k chamadas).
- Total acumulado da sessão (original -> final): **1,17x** no `tools/perf/bench` (sieve 1,52x, forin 1,32x, floats 1,30x, loop_arith 1,21x).

**Não mexido (candidatos):** `StrIndexOf` (varre posição a posição; `s.find` seria mais rápido mas `indexOf("", "")` hoje devolve -1 em
string vazia, e `find` devolveria 0: preservar), `Split/Replace/ToUpper/...` (alocam por natureza), `Len` de string é `chars().count()` (O(n)).

**Testes:** `ok/builtins_fast.spg` (cobre todos os builtins do caminho rápido, valores conferidos à mão), `ok/str_chars_utf8.spg`
(charAt/substring ASCII e UTF-8), `err/{charat_oob,charat_neg,charat_oob_utf8,substring_oob_utf8,substring_range,substring_neg}.spg`.
Provados por sabotagem (Len em bytes, charAt devolvendo o byte 0, substring UTF-8 deslocado, erro do charAt em bytes, pop que não
remove, floor->ceil). `ok/str_chars.spg` (o ORIGINAL, que já cobria UTF-8) continua passando.

**Armadilhas desta sessão:** (1) ANTES de criar um arquivo de teste, conferir se o nome já existe (`ls examples/multi/ok | grep nome`):
sobrescrevi o `ok/str_chars.spg` original sem querer e só percebi pelo `want:` da suíte; restaurado do zip. (2) O terminal é `sh`:
`diff <(...)` só funciona dentro de `bash -c`. (3) Ao recortar blocos de código por "primeiro `}` com N espaços", lembrar que
`"            }\n"` também casa o final de `"                }\n"` (indentação maior).

## Sessão de velocidade 5 (2026-10-03) — stdout bufferizado também no terminal, com thread de flush

Substitui o desenho da Sessão 2 (que deixava o terminal linha a linha). Agora `print` escreve num `BufWriter` de 64 KB **global**
(`static OUT: Mutex<Option<BufWriter<Stdout>>>` em `builtins.rs`), para terminal E pipe/arquivo. Uma thread `spg-flush`
(criada no 1o `print` via `Once`) acorda a cada `FLUSH_EVERY` = 10 ms e descarrega o que estiver pendente: nada fica retido
mais que isso, mesmo se o programa ficar calculando em silêncio.
- Flush SÍNCRONO (`flush_stdout`, espera o lock) antes de `readLine`, `printError`, `@time.sleep` e no fim de `Vm::run` (ok ou erro,
  antes do CLI imprimir "runtime error"). **O flush do fim do `run` é essencial: `static` não é destruído no exit** (sem ele, 30 testes
  falham por saída perdida).
- Hook de panic do CLI usa `flush_stdout_in_panic` (`try_lock`, nunca bloqueia): o hook roda ANTES do unwinding e a thread que entrou
  em pânico pode ainda estar segurando o lock; um `lock()` ali travaria. O `panic!` do `print_line` acontece DEPOIS de soltar o lock.
  Mensagem do erro de escrita = a do `println!` (o hook sai quieto em "Broken pipe").
- Medido (pty = terminal de verdade): 100k prints **183 ms -> 30 ms (6x)**; 1M prints p/ pipe 1055 -> ~157 ms (Sessão 2).
  Latência do 1o print enquanto o programa calcula: terminal 2 -> 13 ms; **pipe 1520 ms -> 16 ms** (a versão da Sessão 2 só mostrava
  o 1o print no fim do programa quando a saída era pipe).
- Limitação: Ctrl+C / abort por stack overflow perdem no máximo os últimos ~10 ms de saída.
- Testes: `ok/print_stress.spg` (300 mil linhas, `lines`: sobrepõe vários ciclos da thread de flush; md5 idêntico em 5 execuções) e
  `timing/flush_latency.spg` (pasta fora da varredura `same`; Python mata o programa assim que o 1o `print` chega ao pipe, ~15 ms;
  exige o 1o chunk == `one\n`, programa ainda rodando, < 1 s; pula com aviso se não houver `python3`). Provados por sabotagem: sem a
  thread (latência falha), thread que descarta o buffer (estresse falha: 279030 de 300000 linhas), `flush_stdout` virando no-op (30 falhas).
- Total acumulado da sessão (original -> final): **1,17x** no `tools/perf/bench`.

**Ainda aberto:** (4 descartado, ver acima) `FieldSet` compostos (`s.f += v`) com o mesmo tratamento dos índices, `StrIndexOf`
(preservar `indexOf("", "")` = -1 em string vazia), `Len` de string é O(n).

## Candidato 4 (`OpCode` 32 -> 16 bytes): MEDIDO PELO TETO e descartado (2026-10-03)

Em vez de refatorar, medi o inverso num laboratório (cópia do projeto em `/tmp/lab`, fora do repo): uma variante-padding
`LabPad([u64; 7])` nunca emitida força `size_of::<OpCode>()` de 32 para **64** bytes; o controle de 32 bytes leva o mesmo arm extra
no `match` (necessário porque qualquer arm novo já oscila +-5%). Conjunto completo: controle 11,751 s vs 64 bytes 11,794 s
(**+0,4% no total, ruído**). Só o laço mais apertado sente: `loop_arith` 1044 -> 1150 ms (**+10%**, 11 rodadas); `floats`,
`collatz`, `sieve`, `structs` etc. sem diferença. Logo, encolher para 16 bytes renderia no máximo ~5% em laços aritméticos
apertados e ~0-1% na média, e exigiria slots `u32`, imediatos `i64` no pool de constantes e mexer em chunk/peephole/compiler/vm:
**custo e risco altos para ganho pequeno, NÃO fazer** a menos que apareça um benchmark dominado por laço aritmético puro.
Método útil para futuras decisões "vale refatorar?": medir o efeito INVERSO (piorar a propriedade) num `/tmp/lab` com controle.
Melhor aposta restante: `FieldSet` compostos (`s.f += v`) como as fusões de índice (o benchmark `structs` é o único que não melhorou).

## Sessão de velocidade 6 (2026-10-03) — fusões de campo de struct (`s.f += v`, `s.f = v`)

Feito num laboratório (cópia em `/tmp/lab`); depois só os arquivos alterados foram copiados de volta (peephole.rs, chunk.rs, vm.rs,
lib.rs, index_ops.rs, **field_ops.rs** novo, `ok/field_fused.spg`, run.sh, gen.py).

Custo por operação ANTES (3M iterações em função, laço vazio ~13 ns/iter): `s.b = 2` ~20 ns, `s.a += 1` ~24 ns, `s.a += x` ~32 ns.
**Adotado** (corpos fora de linha em `spg-vm/src/field_ops.rs`; arms de uma linha no `step`; variantes no FIM do `enum OpCode`):
- `FieldAddConst{local,field_index,k}` = `FieldGetDup; AddConst(k); FieldSet` (`s.f += k` e `-= k`): 113 -> 40 ms.
- `Field{Int,Float}{Add,Sub,Mul}{local,field_index,x}` = `FieldGetDup; GetLocal(x); <op>; FieldSet`: ~136 -> ~44 ms.
- `FieldSetLocalLocal{local,field_index,v}` (`s.f = v`) e `FieldSetLocalConst{...,c}` (`s.f = <const>`): 99 -> ~50 ms.
- Só funde com struct em LOCAL e valor local/constante; o padrão exige o mesmo `field_index` na cabeça e no `FieldSet`.
- `structs` (benchmark): **1,10 s -> 0,92 s (1,20x)**, o único que ainda não tinha melhorado. Semântica de referência preservada
  (`let u = s; u.a += 1` altera `s`; testado).

**Custo/ruído (importante):** `loop_arith` fica ~+4% a +9% MAIS LENTO com qualquer arm novo no `step`, embora o BYTECODE dele seja
IDÊNTICO (confirmado por diff do dump): é layout do código nativo do `step()`. Testado e SEM efeito: variante enxuta com 4 opcodes
(mesmo custo), mover os arms para o fim do `match`. Mover as variantes novas para o fim do `enum` ajudou um pouco (879 -> 850 ms,
base 814 ms), dentro do ruído. `collatz` e os demais ficam iguais. Total do conjunto ~1,02x nesta etapa (o ganho do `structs`
compensa o `loop_arith`). Se algum dia houver um benchmark dominado por laço aritmético puro, considerar PGO ou reduzir arms.

**Testes:** `ok/field_fused.spg` (valores conferidos à mão: os 9 opcodes aparecem no dump). Sabotagens provadas: `+k -> -k`, `int_sub -> +`,
`float_mul -> +`, constante gravada como 0, valor lido do local errado, arm `FieldIntAdd` chamando `int_mul`; e o FUZZ SOZINHO detecta
`int_sub -> +` (6 divergências em 120). `tools/perf/fuzz/gen.py`: struct `S` agora tem `f: float`, há `float` local também nas funções
(`lfv`), declarações de campo/índice com operandos simples e mais peso nos arrays `float`; cobertura medida: todas as fusões de
campo e de índice (inclusive `float`) aparecem em 3-44% dos programas.

**Armadilha:** o lab tinha um binário velho (`f2`) enquanto o código-fonte era outro: SEMPRE recompilar depois de restaurar o código
de um snapshot, senão a "cobertura 0" engana. `sab.sh` aponta para o projeto real; para o lab usar `/tmp/sab_lab.sh`.

**Ainda aberto:** `FieldSet` com valor = campo de struct (`v.x += v.y`, o padrão do benchmark `structs`; pede opcode com 4 campos `u32`),
`StrIndexOf` (preservar `indexOf("", "")` = -1 em string vazia), `Len` de string é O(n), PGO.

## Sessão de expansão da linguagem (2026-10-04) — CONCLUÍDA (v0.3.0 → v0.4.0)

Três recursos novos, pedidos pelo usuário. Tipos novos (`map`, `enum`, `char`...) ficaram para depois, por decisão dele.

### 1. `'...'` — string CRUA (só no lexer)
- `'print("hi")'` é uma string que contém aspas duplas. SEM escapes de nenhum tipo (`'a\nb'` tem 4 chars).
- Fecha na PRIMEIRA `'` seguinte e nunca olha adiante; por isso `'print('hi')'` falha de propósito
  (vira `'print('`, `hi`, `')'` e o parser rejeita; com número ímpar de aspas dá `unterminated string`
  apontando a aspa que abriu). O parser não tem tratamento especial. `"..."` não mudou.
- Código: `Lexer::read_raw_string`.

### 2. Ranges: `for i in a..b [step s]` e `@array.range(a, b[, step])`
- Fim **EXCLUSIVO** nas duas formas; passo negativo conta pra baixo. `a..b` só existe no cabeçalho do `for`
  (`let r = 1..4` dá erro explicando e apontando `@array.range`). `step` é palavra CONTEXTUAL (não reservada).
- Lexer: token `DotDot` (`1..4` lexa como `1`, `..`, `4`; `1.5` continua float).
- AST/typed: `Stmt::ForRange` / `TypedStmt::ForRange { .., step: RangeStep, var_assigned, .. }`.
  `RangeStep` = `One` | `Const(i64)` (literal, sinal conhecido) | `Dynamic(expr)` (sinal/zero checados em runtime).
  start/end/step avaliados UMA vez, na ordem; os três têm de ser `int`. Passo literal 0 = erro de tipo.
- Compilador (`compile_for_range`): laço de contador SEM array. A direção (`<` ou `>`) é fixada em compile time
  para passo ausente/literal (mesmo formato `compare+JumpIfFalse` de um `while`, a peephole funde). Passo dinâmico:
  builtin interno `RangeStepCheck` (erro `ZeroRangeStep` se 0) + teste de sinal em runtime. `continue` cai no incremento.
- **Variável do laço sem cópia quando ninguém a reatribui** (`VarInfo.assigned` no checker). Sem isso `a..b` ficou 1,5x
  mais lento que `while` (350 vs 230 ms em 20M iterações); com isso empata (~216 ms). Se o corpo faz `i = ...`,
  o corpo ganha uma CÓPIA do contador (reatribuir não muda a iteração, como no Python).
- `@array.range` (BuiltinId `ArrayRange2/3`): `i128` por dentro (sem overflow nos extremos de `i64`); passo 0 = erro de
  runtime; mais de `MAX_RANGE_LEN` (20 milhões) elementos = erro de runtime em vez de abortar por falta de memória.
  Fica em `@array` e NÃO em `@math` (devolve array).

### 3. Métodos de struct: `p.f(x)`, com `me` implícito
```
struct Counter {
    n: int

    function tick() -> Counter { me.n += 1  return me }
}
```
- Corpo do struct mistura campos (separados por vírgula; vírgula final ok) e métodos (sem separador). `me` é declarado
  AUTOMATICAMENTE como 1º parâmetro (NÃO aparece na assinatura); nome escolhido pelo usuário no lugar de `self`.
  Campo solto (`x` em vez de `me.x`) NÃO é aceito (colidiria com globais). `me` é identificador comum fora de métodos.
- **Desenho (zero opcode novo):** o PARSER baixa cada método para uma `FunctionDecl` comum chamada `P.f` com
  `me: P` na frente (fila `Parser.pending`, drenada logo depois do struct por `parse_program`/`parse_block`). O `.` no
  nome nunca colide com nada que o usuário escreva. O TYPECHECK (`check_method_call`) reescreve `obj.f(args)` em
  `P.f(obj, args)`; o receptor é o argumento 0 (avaliado uma vez, antes dos outros). A VM nunca soube de métodos.
- **Multi-arquivo:** o mangling dos módulos casa sozinho: struct `tag::P` + método `P.f` = `tag::P.f`, e o typecheck monta
  `format!("{}.{}", struct_mangled, method)`. Dois structs homônimos em arquivos diferentes, cada um com seu `len2`, funcionam.
  O nome mostrado em erros passa por `display_name` (nenhum `tag::` vaza). Quem muda o formato do nome quebra tudo (7 testes).
- Erros novos: `NoSuchMethod` (com variante "é um campo, não um método"), `NoMethodsOnType` (`5.get()`), `MethodNotCalled`
  (`p.get` sem chamar), `FieldMethodClash` (parser), aridade/tipo de argumento contando só o que o chamador escreveu (sem `me`).
- Limites atuais: sem métodos estáticos (sem `me`), sem herança/interface; chamar `f` de outro arquivo só via valor do struct.

### 4. `print` / `printError` variádicos (feito depois, mesma sessão)
- `print()` imprime uma linha vazia; `print(a, b, c)` imprime os valores separados por UM espaço (como o Python), qualquer
  tipo, qualquer quantidade. `printError` segue a mesma regra. Antes exigiam exatamente 1 argumento.
- Typecheck: caso especial em `check_expr` (`Expr::Call` de builtin) para `BuiltinId::Print | PrintError`: aceita qualquer
  aridade e nunca dá `ArityMismatch`; a assinatura de 1 `Any` na tabela serve só de identidade. VM: `write_joined` (1 argumento
  mantém o `writeln!` original, então o caso comum custa o mesmo; medido por alternância: empate ~300-320 ms em 3M prints).
- Teste `ok/print_variadic.spg` (stderr entra na captura, ordem garantida pelo flush do stdout antes do `printError`);
  provado por 5 sabotagens (vírgula no lugar do espaço, `print()` vazio, `printError` só com o 1º argumento, ordem invertida,
  aridade checada de novo).

### Editor e instalador
- LSP: `@array.range` aparece sozinho (vem da tabela de builtins). Corrigido: métodos NÃO aparecem como funções livres no
  autocompletar (rastreia profundidade de `{}` pra saber que está dentro de `struct`). Teste dedicado, provado por sabotagem.
- Gramática (`spg.tmLanguage.json`): `'...'` (regra `raw-strings`, sem escapes), operador `..` (e o ident depois dele NÃO
  vira propriedade: o `member` exige `(?<=[^.]\.)`), `step` contextual (`\bstep\b(?=\s+[-(A-Za-z_0-9])`, então
  `let step = 2` continua variável), `me`. Validada com `vscode-textmate` + `vscode-oniguruma` (npm, em /tmp).
- Snippets `forr` e `structm`; `'` no autoClosingPairs/surroundingPairs.
- Versão 0.4.0 em `Cargo.toml` (workspace), `editors/vscode/package.json`, README, comentário do `install.ps1` e check
  `^spg 0\.4\.0` em `tools/installer-tests/run.ps1`. **O instalador compara a versão da extensão**: `.vsix` novo com a mesma
  versão seria tratado como "já instalado", por isso o bump. `dist/spaghetti-0.4.0.vsix` (ícones idênticos ao anterior,
  conferido por md5). Empacotar: `npm i @vscode/vsce` em /tmp e `vsce package --no-dependencies --allow-missing-repository --skip-license`.
- Mensagem final do instalador agora diz `@math.` (antes `math.`).
- Os testes do instalador (PowerShell) NÃO rodam no container (sem `pwsh`); só a mudança de texto/versão foi feita neles.

### Testes e medição
- Suíte multi-arquivo 150 → **191**: `ok/raw_strings`, `ok/range_for`, `ok/struct_methods`, `ok/struct_methods_multi`
  (+ `lib/sm_vec.spg`, `lib/sm_other.spg`), `err/range_*`, `err/raw_*`, `err/method_*`, `err/not_callable_expr`, `diag/method_mangled_*`,
  e os exemplos `examples/{methods,ranges,raw_strings}.spg` (entram na checagem de equivalência com/sem `--no-opt`).
- `run.sh` agora roda cada programa sob `timeout 30` (função `spgrun`): laço infinito vira `FAIL (exit=124)` de UM teste.
  `SPG` continua sendo o caminho puro (o script Python do teste de flush e outros usos entre aspas dependem disso).
- Provado por sabotagem: raw com escape; var do laço sem cópia; direção do passo literal e do dinâmico; tamanho do
  `@array.range` (floor em vez de ceil); passo 0 literal e dinâmico aceitos; método: lookup por nome exibido, aridade contando `me`,
  separador do nome, colisão campo×método, dica "é um método", tipo de `me`, `is_field`; LSP: método vazando.
  **NÃO provado:** o limite de 20 milhões do `@array.range` (arriscado simular sem estourar memória).
- Performance: `fib`/`fact`/`call_2arg` empatam com o binário original em alternância de 5 rodadas.
- Compilou offline com `CARGO_HOME` vazio (zero dependências).

## Sessão `install` (2026-10-05) — CONCLUÍDA (v0.5.0 → v0.6.0)

Ideia do usuário: `install "./file.spg"` carrega TODOS os exports do arquivo sem `import { ... }` enorme. `import { x }` continua para itens específicos.
- **Regras aprovadas**: prioridade nome declarado no arquivo > `import { x }` explícito > `install`. Importar e instalar o MESMO arquivo não é conflito.
  Dois `install`s dando o mesmo nome com significados DIFERENTES só erram se o nome for USADO (`AmbiguousInstall`, cita os dois arquivos/linhas; resolver: `import { x }`).
  Mesma declaração alcançada por dois caminhos (ex.: via re-export) não é ambígua. Erros do `import` ficaram intocados. Privado (sem `export`) nunca entra.
- **Como**: palavra reservada `install` (lexer) -> `Stmt::Install { from, pos }` (parser) -> só o `spg-modules` conhece: passo 2b em `lib.rs` roda DEPOIS dos imports (para saber o que já está ocupado),
  usa `resolve_all` (exports + re-exports do arquivo, ordenados por nome). `resolve_names` foi dividido em `load_dep` + `pick_names` (comportamento igual). A ambiguidade NÃO entra no `name_map`;
  o `Renamer` registra o USO (`with_ambiguous` / `used_ambiguous`, em `value_name` e `type_name`) e o resolvedor erra depois do rename. Nada novo no typecheck/compilador (`unreachable!` como no `ReExport`).
- **Editor**: `install` em `KEYWORDS` do LSP, gramática e snippet; `.vsix` 0.6.0.
- **Testes (enxutos, a pedido)**: `ok/install_{basic,priority,unused_clash}`, `err/install_{clash_used,private,no_ext}`, `lib/in_{a,b,facade}.spg`. SEM sabotagens nesta sessão (preferência dele: testar o suficiente).
- **v0.6.1 — autocomplete de nomes de outros arquivos (LSP)**: `import { a, b } from "x"` oferece a e b; `install "x"` oferece TODOS os exports de x (função/struct/enum/let/const com o ícone certo,
  `detail` mostra "from x"/"installed from x"). Feito lendo os tokens dos arquivos (`exports_of` em `server.rs`: `export function/struct/enum/let/const NOME` + `export { a } from`), sem resolver o grafo,
  então funciona com o buffer meio digitado e um buffer aberto ganha do disco. Teste: `completion_offers_names_from_import_and_install` (LSP 29 -> 30). Limite conhecido: nome vindo de re-export aparece com ícone de função.
- (FEITO na v0.8.1, ver "Sessão autocomplete de structs".) Structs DECLARADOS no arquivo já eram oferecidos como nome (ícone struct).

## Sessão `enum` (2026-10-05) — CONCLUÍDA (v0.4.0 → v0.5.0)

Base conferida verde antes de mexer (193/193). Sem opcode novo.

### O que existe
- `enum Color { Red, Green, Blue }` (vírgulas; vírgula final ok; quebra de linha livre), `export enum`, uso `Color.Green`.
  Variantes: nome único dentro do enum (`DuplicateVariant`, aponta a posição da repetida) e pelo menos uma (`EmptyEnum`).
- Tipo DISTINTO de `int` e de outros enums: só `==` / `!=` entre o MESMO enum; `<`, `+`, comparar com `int`, atribuir a `int`, `@into.int(c)`,
  `c.campo` e `c.metodo()` são erros de tipo. `enum` virou palavra reservada.
- `for c in Color { ... }` percorre as variantes na ordem de declaração. `Color.Purple` -> erro listando as variantes; `let x = Color` -> erro `EnumAsValue`.
- `print(c)`, `print(arr)`, `print(struct)`, `@into.string(c)` mostram o NOME da variante (também dentro de array/struct).
- Um local/parâmetro/variável com o MESMO nome do enum sombreia o enum (`enum_named` checa `lookup` antes), como qualquer nome.

### Como foi feito (para não refazer)
- **Parser**: `Stmt::EnumDecl { name, variants, exported, pos }`, `parse_enum_decl`. **Módulos**: entra em `annotate` (carimba pos), `rename` (nome mangled como struct;
  variantes NUNCA são renomeadas — só existem como `Enum.Variante`) e na tabela de símbolos exportáveis (`lib.rs`). Re-export/import funcionam como struct.
- **Checker**: `enum_defs: HashMap<nome, Vec<variantes>>` ao lado de `struct_defs`. O tipo de um valor é `Ty::Named(nome do enum)` (sem `Ty` novo);
  `is_enum_ty` diferencia de struct. `Expr::Field` sobre `Ident` que é enum -> `TypedExpr::EnumLit`. `for` sobre `Ident` que é enum -> `ArrayLit` de `EnumLit`
  (o resto do `for` não muda). `==`/`!=` entre enums embrulha os dois lados em `TypedExpr::AsInt` (não emite nada) para os opcodes int servirem.
  `@into.string` sobre enum escolhe `BuiltinId::EnumToString` (fora da tabela de overloads).
- **VM**: `Value::Enum(u32 enum, u32 variante)` (8 bytes; `Value` não cresceu). `Program.enum_defs` + `thread_local ENUM_DEFS` em `value.rs`
  (`install_enum_defs`, chamado por `Vm::new`) porque o `Display` não enxerga a VM e precisa do nome mesmo dentro de array/struct.
  `expect_int` e `local_int` aceitam `Value::Enum` (braço frio) -> as fusões `EqIntLocals`/`JumpUnlessEqLocals`... valem sem mexer no hot path.
  `PartialEq` de `Value` compara os dois campos (`@array.contains`/`indexOf` funcionam).
- **Compilador**: passo 1b' numera os enums; `EnumLit` -> `Const(Value::Enum(idx, variante))`; `AsInt` compila só o interior.
- **Editor**: `enum` em `KEYWORDS` do LSP; `Color.` (enum declarado NO documento, achado relendo os tokens) lista só as variantes; gramática TextMate ganhou
  `enum Nome`; snippet `enum`. `.vsix` 0.5.0 reempacotado.

### Testes e provas
- Suíte 193 -> **219**: `ok/enum_{basic,forms,multi,scope}` (+ `lib/en_{color,facade,other}.spg`, dois enums `Color` em arquivos diferentes), 14 `err/enum_*`,
  3 `diag/enum_mangled_*`, `examples/enums.spg`; LSP 28 -> 29 testes. Os `ok` entram na checagem otimizado x `--no-opt`; `enum_forms` confirma que as fusões disparam.
- Provado por sabotagem: índice da variante deslocado; `Display` imprimindo o índice; `expect_int` lendo enum como 0 (`==` sempre verdadeiro -> suíte trava, exit=124);
  `local_int` lendo enum como 0 (trava `enum_forms`, exit=124); sombreamento ignorado; `for` em ordem invertida; LSP que nunca acha o enum.
  **NÃO provado por sabotagem:** a mensagem estrutural de `DuplicateVariant`/`EmptyEnum` (os `err` conferem o texto e a posição, não há sabotagem do parser).
  Armadilha repetida: a primeira rodada de sabotagens estourou o limite de 300 s do comando no meio — rodar UMA sabotagem por chamada e, depois de qualquer
  estouro, restaurar o snapshot (`diff -r`) antes de continuar.

### Detalhes que podem surpreender
- Comparar dois enums DIFERENTES com o mesmo nome em arquivos distintos mostra "operator Eq not supported for Color and Color" (nomes exibidos sem o prefixo do módulo).
- `DuplicateTypeName` foi escrito e REMOVIDO: struct+enum de mesmo nome já são barrados pelo resolvedor de módulos ("'P' is declared more than once").
- (SUPERADO na v0.8.0: agora há valores, ver "Sessão `enum` com valores".) `@into.int(c)` continua erro; use `c.value`.

## v0.6.2 — ícones novos da extensão (2026-10-05)

- `editors/vscode/images/icon.png` (720x720) e `file-icon.svg` trocados pelos do `imagesSpaghetti.zip` dele (tom levemente ajustado, handle do garfo mais grosso). Nomes e tamanhos idênticos.
  O SVG novo veio SEM `viewBox`; foi adicionado `viewBox="0 0 720 720"` (igual ao antigo) senão não escala no VSCode. md5 dos arquivos dentro do `.vsix` conferido contra os originais.
- Bump 0.6.1 → 0.6.2 (Cargo.toml/Cargo.lock, package.json, README, comentário do install.ps1, check `^spg 0\.6\.2` em `tools/installer-tests/run.ps1`, que estava velho em 0.4.0). Necessário porque o instalador compara a versão da extensão.
- Código da linguagem intocado: suíte 228 ok.

## Sessão `map` (2026-10-05) — CONCLUÍDA (v0.6.2 → v0.7.0)

### O que existe
- Tipo `map<K, V>` (`map` só é tipo quando seguido de `<`: uma variável chamada `map` continua valendo). Chaves: `int`, `string`, `bool` ou enum (nunca `float`/array/struct). Valor: qualquer tipo.
- Literal `["ana": 30, "bia": 25]` e `[:]` (vazio). Chaves/valores do literal têm de ser homogêneos (erro com linha/coluna do literal). Chave repetida no literal: a ÚLTIMA vence.
- `m[k]`, `m[k] = v` (insere ou sobrescreve; sobrescrever mantém a posição), `m[k] += v` (e `-=`, `*=`...). **Ler chave ausente = erro de runtime** (`key not found in map: "zz"`), também no `+=`: contar palavras exige `if @map.has(m, w) { m[w] += 1 } else { m[w] = 1 }`.
- `@map.has(m, k)`, `@map.get(m, k, reserva)`, `@map.remove(m, k)` (devolve `bool`: existia?), `@map.len(m)`, `@map.keys(m)`, `@map.values(m)` (arrays, na ordem de inserção). Nova namespace `@map` (`NAMESPACES` agora tem 6).
- Ordem de inserção sempre (print, keys, values). `print(m)` mostra `[ana: 31, bia: 26]`, vazio `[:]` (strings sem aspas, igual ao array). Maps são por referência (`let b = a` compartilha), como array/struct.
- `for k in m` NÃO existe (decisão: `for k in @map.keys(m)`); `len(m)` global também não (use `@map.len`).

### Como foi feito (para não refazer)
- **Parser**: `TypeExpr::Map`, `Expr::MapLit(Vec<(Expr,Expr)>)`; `[` seguido de `:` = mapa vazio; um `:` depois do 1º elemento vira literal de map. **Módulos**: `annotate` e `rename` ganharam `MapLit`; ARMADILHA PEGA: `rename_type` tinha `_ => {}` e engolia `map<string, Ponto>` (struct importada não era renomeada) — agora tem braço `Map` explícito (teste `ok/map_multi` + `diag/map_mangled`).
- **Typecheck**: `Ty::Map(K,V)` e `Ty::AnyMap` (só builtin). **`Ty` perdeu o `derive(PartialEq)`**: o `==` manual trata o tipo do `[:]` (`Map(Void,Void)`, `is_empty_map`) como IGUAL a qualquer `map<K,V>`; assim `[:]` passa em anotação, argumento, return, atribuição e campo sem propagar "tipo esperado". `let m = [:]` sem anotação é erro (`EmptyMapNeedsType`). `index_target` único para `m[i]`/atribuição/composta (array: índice `int`; map: tipo da chave). `@map.*` têm assinatura-placeholder na tabela e `check_map_call` valida chave/reserva contra o map real e devolve `K`/`V`/`array<K>`/`array<V>`. Chave inválida só é barrada no USO (índice/builtin/literal): `let m: map<float,int> = [:]` sozinho passa.
- **Compilador**: `OpCode::MakeMap(n)`, `MapGet`, `MapSet` (sem opcode de fusão: o peephole só casa `IndexGet`/`IndexSet`, então as fusões de array nunca veem um map). `m[k] += v` = `Dup2; MapGet; v; op; MapSet`.
- **VM**: `Value::Map(Rc<RefCell<SpgMap>>)`; `SpgMap` = `HashMap<MapKey, usize>` + `Vec<(Value,Value)>` (ordem de inserção). get/insert O(1); **`remove` é O(n)** (reindexa o que vem depois). `MapKey` = Int/Str/Bool/Enum. `structurally_equal` de map ignora a ordem. `RuntimeError::KeyNotFound`.
- **Editor**: `@map` na gramática, `map` como tipo só antes de `<`, snippet `map`, `map` na lista de tipos do LSP (`@map.` lista os 6 membros automaticamente pela tabela).

### Testes
- Suíte 228 -> **246**: `ok/map_{basic,forms,multi}` (+ `lib/mp_types.spg`), 11 `err/map_*` (inclui chave ausente e `+=` em chave ausente), `diag/map_mangled`. LSP 30 -> 31. Os `ok` entram na checagem otimizado x `--no-opt`. SEM sabotagens (preferência dele: testar o suficiente).
- Mudou de propósito: o texto de `err/ns_desconhecido` (lista de namespaces agora inclui `@map`).

### Achado FORA do escopo — CORRIGIDO na v0.8.0 (ver "Sessão `enum` com valores")
- `a == b` entre dois ARRAYS passa no typecheck mas o compilador dá panic (`compiler bug: binary Eq on Array(Int) passed typecheck but has no opcode`). Também `[]` vazio não é tipável (`let a: array<int> = []` -> "declared type is array<int> but value is array<void>"); o truque do `[:]` (Ty::eq) resolveria igual para arrays se ele quiser.

## Sessão `enum` com valores + arrays (2026-10-06) — CONCLUÍDA (v0.7.0 → v0.8.0)

Decisões DELE: nome de variante repetido = erro; VALOR repetido é permitido; valores de qualquer tipo (int, float, bool, string).

### Enum com valores
- `enum Ns { one = 1, two = 2, three = 2 }`, `Name { A = "alfa" }`, `Flag { On = true }`, `Ratio { Half = 0.5, Neg = -1.5 }` (negativos ok). Só LITERAIS (nada de `1 + 2` nem nome de variável).
- Todas as variantes de UM enum têm o mesmo tipo de valor (misto = erro de parse). Sem nenhum `=`: ints 0, 1, 2... Depois de um int, a variante sem `=` vale o anterior + 1 (`{ Low, Mid = 10, High }` -> 0, 10, 11). Enum não-int exige `=` em toda variante (`needs a value`). Variante sem `=` conta como int para a checagem de tipo (`{ A, B = "x" }` é erro).
- Acesso: **`c.value`** (tipado: `int`/`float`/`string`/`bool` conforme o enum), também `Color.Red.value` (vira constante em compile time). `print(c)` e `@into.string(c)` continuam mostrando o NOME da variante. `@into.int(c)` continua erro (um acessor genérico `.value` serve a todos os tipos).
- `==` entre enums compara a VARIANTE, não o valor: `Ns.two == Ns.three` é `false` mesmo com o mesmo valor; para comparar valores, `Ns.two.value == Ns.three.value`.
- Como foi feito: o PARSER resolve a numeração automática (`Stmt::EnumDecl.values: Vec<EnumValue>`, um por variante, `ast::EnumValue`), então o resto vê sempre um valor por variante. Erros novos de parse: `EnumValueType`, `EnumValueNeeded`. Checker: `enum_value_tys` (nome do enum -> `Ty`); `Expr::Field` com campo `value` num valor de enum -> `TypedExpr::EnumValue`. Compilador: `EnumDef.values` (na VM) e `StructTable.enum_values`; `EnumValue` sobre `EnumLit` vira `Const`, senão opcode `EnumValue` (lê `program.enum_defs[def].values[variante]`).
- Não há conversão inversa (valor -> enum) nem `.name`.

### `array == array` (panic antigo do compilador)
- `==`/`!=` entre arrays, maps e structs agora compara por CONTEÚDO (profundo): opcodes `EqValue`/`NotEqValue` usando `Value::structurally_equal`. O typecheck já aceitava; só faltava o opcode no `binary_opcode`.

### `[]` vazio
- `let a: array<int> = []`, `return []`, `f([])` funcionam: `Ty::eq` trata `array<void>` (tipo de `[]`) como igual a qualquer `array<T>` (mesmo truque do `[:]`). `let a = []` SEM anotação continua `array<void>` como sempre (não virou erro, ao contrário de `let m = [:]`). Efeito colateral: quem declara `array<void>` à mão passa a aceitar qualquer array (inútil na prática).

### Testes
- Suíte 246 -> **254**: `ok/enum_values`, `ok/array_eq_empty`, 4 `err/enum_val_*`, + 2 checagens otimizado x `--no-opt`. Verificado à mão: enum com valores exportado/importado entre módulos. SEM sabotagens (preferência dele).

## Sessão autocomplete de structs (2026-10-06) — CONCLUÍDA (v0.8.0 → v0.8.1)

Só LSP (o `spg.exe` leva o código; a extensão não mudou além do número da versão). Módulo novo `crates/spg-lsp/src/members.rs` + cola em `server.rs`.

### O que o editor faz agora
- **`p.`** (e `p.na`): oferece os CAMPOS (ícone campo, detalhe = tipo) e os MÉTODOS (ícone método, detalhe = `nome(params) -> ret`) do struct de `p`. Vale para: `let p = Struct { ... }`, `let p: Struct`, `let p = f()` (tipo de retorno de função livre), parâmetro `p: Struct`, `catch (e: T)`, global (`let`/`const` no topo), **`me.`** dentro de método, cadeias `a.pos.` (segue o tipo do campo) e structs vindos de `import`/`install` (lê os tokens do outro arquivo; buffer aberto vence o disco).
- **Enum**: variável do tipo enum (`let c = Color.Red`, param `c: Color`) e `Color.Red.` oferecem `value` (a feature da v0.8.0). `Color.` continua listando as variantes (já era assim; só enum do próprio arquivo).
- **`Struct { `**: em posição de NOME de campo (logo depois de `{` ou `,`, ignorando quebras de linha) oferece os campos que AINDA NÃO foram escritos, com `insertText` `campo: `. Em posição de valor (depois de `campo:`), dentro de chamada/índice, na declaração `struct X {` e em `-> X {` não oferece isso (volta à lista normal).

### Como funciona (heurística em TOKENS, de propósito)
- O buffer está quebrado nesse momento (`p.` sem nada depois não parseia), então NÃO usa parser nem typecheck: `Decls::from_tokens` monta a tabela (structs com campos/métodos, enums, funções livres -> tipo de retorno); `resolve_head` acha o tipo declarado do nome: locais da função que contém o cursor (do mais próximo pro mais distante) -> parâmetros -> globais; `for x in ...` conta como "declarado, tipo desconhecido" (para shadowing correto).
- Quando não sabe, não oferece NADA (ex.: `arr[0].`, `f(x).`, `array<Struct>`, variável de laço, nome não declarado, int...). `it.` sobre um `array<P>`/`map<K,P>` indexado NÃO é resolvido (limite conhecido; o receiver tem que ser cadeia de nomes simples).
- Token `line`/`col` são 1-based; o cursor do LSP é 0-based; o "word" parcial sendo digitado fica fora da contagem de tokens.

### Bug achado e corrigido no caminho
- `line_before_cursor` usava `text.lines()`, que DESCARTA a última linha vazia: com o cursor logo após um `\n` final (Enter dentro de `Struct { a: 1,`), a completion falhava e caía na lista genérica. Agora usa `split('\n')` (e tira `\r`).

### Testes
- LSP 31 -> **38**: 6 unitários em `members.rs` (tabela, local/param/global/me, cadeia+enum, laço/desconhecido, shadowing, literal) + 1 ponta a ponta pelo protocolo com struct IMPORTADO (`it.` -> name, qty, total com os ícones certos; `Item { name: "y", ` -> só `qty` com insertText). Sem sabotagens (preferência dele). Não testado numa janela real do VSCode (só pelo protocolo).

## Sessão `@array`/`@math` completos (2026-10-06) — CONCLUÍDA (v0.8.1 → v0.9.0)

Pedido dele: completar os namespaces JÁ existentes (`@math.*`, `@array.*`) antes de novos (`@file`, `@sys`, `throw`...).

### `@array` (existia: contains, indexOf, len, push, pop, range, slice, reverse, sort, join)
- Novos: `insert(a, i, x)` (i de 0 a len; fora disso = erro de índice), `remove(a, i)` (POR ÍNDICE, devolve o removido), `clear(a)`, `fill(n, x)` (array novo com n cópias; **x só int/float/bool/string**: array/struct seriam COMPARTILHADOS por referência, então é erro de tipo `ArrayFillType`; n < 0 = `NegativeCount`, n > 20 milhões = `RangeTooLarge`), `concat(a, b)` (array novo, tipos iguais), `sum(a)` (`array<int>` -> int com wrapping como o `+`; `array<float>` -> float; `sum([])` = 0).
- **Buraco de tipos CORRIGIDO**: `@array.push/contains/indexOf` declaravam o elemento como `Any`, então `@array.push(a, "x")` num `array<int>` passava no typecheck e estourava em runtime ("VM bug: expected int"). Agora `check_array_call` (checker.rs, ao lado de `check_map_call`) valida o elemento contra o tipo do array para push/contains/indexOf/insert e dá o tipo de fill/concat. `array<void>` (o `[]` sem anotação) continua aceitando qualquer elemento, como sempre.
- Achado NÃO corrigido: `@array.pop` num array vazio devolve `void` em silêncio (vira "VM bug" se o resultado for usado). Candidato a erro de runtime claro.

### `@math` (existia: abs, floor, ceil, round, sqrt, pow, min, max, random, pi, e)
- Novos (todos `float -> float`, ângulos em radianos): `sin cos tan asin acos atan exp log log2 log10 toRadians toDegrees`, `atan2(y, x)`. Domínio como IEEE, igual ao `sqrt`: `log(0.0)` = `-inf`, `asin(2.0)` = `NaN` (sem erro).
- `sign(x)` (int ou float -> int: -1, 0, 1; NaN -> 0), `clamp(x, low, high)` (int ou float; `low > high` = erro de runtime `ClampBounds`; implementado à mão porque `f64::clamp` dá panic com limite NaN), `tau()` / `tau(n)` (igual ao `pi`: 2 casas por padrão, n casas arredondadas).
- Parâmetros float NÃO aceitam int (`@math.sin(1)` é erro, como o `sqrt` sempre foi): sem widening implícito.

### Onde mexe
- `BuiltinId` novos em `spg-typecheck/src/builtins.rs` (+ `is_array_checked`), `check_array_call` no checker, erro `ArrayFillType`; VM: `NegativeCount`, `ClampBounds` e arms no `call_builtin` genérico (nenhum no caminho quente da pilha). LSP: `@array.`/`@math.` listam os novos sozinhos (a tabela é a fonte).
- README da extensão atualizado (citava só 5 namespaces e "nada depois de `.` de struct").

### Testes
- Suíte 254 -> **263**: `ok/ns_array_math` (todas as funções novas + o `push` tipado), 7 `err/array_*`/`err/math_clamp_bounds`, + 1 checagem otimizado x `--no-opt`. Sem sabotagens (preferência dele).

## Sessão `@str` completo + `pop` + literal de array (2026-10-06) — CONCLUÍDA (v0.9.0 → v0.10.0)

### `@str` (existia: charAt, contains, endsWith, indexOf, len, repeat, replace, split, startsWith, substring, toLower, toUpper, trim)
- Novos: `padStart(s, width [, fill])` e `padEnd(s, width [, fill])` (fill padrão `" "`, repetido e CORTADO até `width` CARACTERES; `width` <= tamanho ou `fill` vazio devolve `s` igual, sem erro; `width` > 20 milhões = `RangeTooLarge`), `lastIndexOf(s, needle)` (índice em CARACTERES, -1 se não achar; igual ao `indexOf`, que também conta caracteres, não bytes), `trimStart(s)`, `trimEnd(s)`.
- `padStart`/`padEnd` são dois ids cada (com e sem `fill`: `PadStart`/`PadStartFill`/`PadEnd`/`PadEndFill`), mesma implementação.

### Dois buracos antigos fechados
- `@array.pop` em array VAZIO devolvia `void` em silêncio (virava "VM bug" ao usar o resultado). Agora é erro de runtime `EmptyArray`: `@array.pop on an empty array` (nos dois caminhos: pilha e genérico).
- Literal de array NÃO checava os itens: `[1, 2.5]` passava no typecheck e estourava em runtime. Agora `ArrayItemMismatch` (`all items of an array must have the same type: expected int, found float`, aponta o literal). `[[], [1]]` continua valendo (o `array<void>` do `[]` casa com qualquer array; o tipo do item vira o que diz algo).

### Testes
- Suíte 263 -> **269**: `ok/ns_str_pop` (todas as funções novas, com acento, + pop + `[[], [1]]`), 4 `err/*` (`array_pop_empty`, `array_lit_mixed_num`, `array_lit_mixed_type`, `str_pad_type`), + 1 `--no-opt`. Sem sabotagens (preferência dele). Instalador compara versão por IGUALDADE, então `0.10.0` não tem problema de ordenação.

### Conversa em aberto: tipo `number` (ideia dele, 2026-10-06)
- A dor real: hoje NÃO há mistura int/float nenhuma (`1 + 2.5`, `10 / 3.0`, `2.0 * 3`, `let y: float = 10`, `h(4)` com `h(n: float)` são todos erro de tipo), então `.0` em todo lugar; e `10 / 3` (int/int) dá 3. Floats inteiros imprimem sem `.0` (`2.0` -> `2`).
- Ideia dele: um tipo `number` (como TS) que serve p/ os dois, `10 / 3` dando 3.333. Ele disse que é só ideia, sem pressa. Análise e opções foram apresentadas no chat (ver resposta); NADA implementado. Decisão pendente dele.

## Sessão métodos nativos (2026-10-07) — CONCLUÍDA (v0.10.0 → v0.11.0)

### O que existe
- `recv.nome(args)` quando `recv` é **string, array ou map** equivale a `@ns.nome(recv, args)` (`@str` / `@array` / `@map`): `s.len()`, `s.trim().toUpper()`, `xs.push(1)`, `xs.sum()`, `xs.concat(ys).reverse()`, `m.has(k)`, `m.get(k, 0)`, `b.tags.push("x")`, `grid["r"].push(2)`, `"abc".len()`, `[1, 2].len()`, `@array.range(0, 4).len()`.
- A forma `@ns.nome(recv, ...)` continua valendo igual. Custo em runtime ZERO: o checker produz exatamente o mesmo `TypedExpr::Call` com o mesmo `BuiltinId` (nenhum opcode novo).
- Só são métodos os builtins cujo PRIMEIRO parâmetro aceita o valor: `xs.range(1, 2)` e `xs.fill(...)` NÃO existem (`range`/`fill` começam com int) -> `NoSuchNativeMethod`. Nomes que não existem no namespace também.
- Não há açúcar para int/float/bool/enum (`n.abs()`, `x.toString()`): continua `NoMethodsOnType`. Possível extensão futura (`@math`/`@into`), não pedida.
- Erros contam só os argumentos que o usuário escreveu (como nos métodos de struct): `xs.push()` -> `'@array.push' expects 1 argument(s), got 0`; o nome mostrado continua `@array.push`.

### Como foi feito
- `checker.rs`: o bloco de builtins de `Expr::Call` foi EXTRAÍDO para `check_builtin_call(name, typed_args, pos) -> Option<TypedExpr>` (o chamador testa `builtin_signatures().contains_key` antes). `check_method_call` ganhou o ramo `Ty::String|Array|Map` -> `check_native_method`, que monta `@ns.nome`, confere `takes_receiver` (primeiro parâmetro de algum overload casa com o receptor), chama `check_builtin_call` e depois desconta o receptor dos índices/contagens dos erros novos (`ArityMismatch`, `ArgTypeMismatch`). Erro novo `NoSuchNativeMethod`.
- LSP (`members.rs`): o tipo de uma variável agora é o TEXTO do tipo (`array<int>`, `map<string, int>`, `string`, nome de struct), não só nomes simples; literais inferem `string`, `array` ou `map` (`[:]` e `["a": 1]` = map, pelo `:` antes da primeira vírgula). `native_methods(ns)` monta a lista pela própria tabela de builtins (primeiro parâmetro aceita o valor), ordenada por nome, detalhe SEM o receptor (`push(any) -> void`). Campos de struct do tipo `array<...>` encadeiam (`b.tags.`). Variável de laço e `f(x).` seguem sem sugestão.

### Testes
- Suíte 269 -> **276**: `ok/native_methods` + 5 `err/native_*` + 1 `--no-opt`. LSP 38 -> **39** (1 teste novo; um velho foi atualizado: campo `array<string>` agora OFERECE os métodos de array). Sem sabotagens (preferência dele).

## Sessão `throw` (2026-10-07) — CONCLUÍDA (v0.11.0 → v0.12.0)

### O que existe
- `throw expr` (statement; `throw` é palavra-chave nova). **O valor lançado é sempre uma `string`** (a mensagem): `throw "negative: " + @into.string(n)`. Outro tipo = erro de tipo (`ThrowNotString`).
- `try { ... } catch (e) { ... }`: `e` é SEMPRE `string`. `catch (e: string)` também vale; `catch (e: int)`/outro tipo = `CatchNotString`. (Antes o tipo padrão era `Named("Error")`, sem estrutura nenhuma: o `try`/`catch` era código morto, nada lançava.)
- A exceção **atravessa funções**: um `throw` dentro de `f()` chamada de dentro de um `try` é pego pelo chamador, em qualquer profundidade e no MEIO de uma expressão (`let x = 100 + risky(-1) * 2`). Relançar de dentro de um `catch` vai para o `try` de fora. `return` dentro de `try` e função que termina em `throw` (`-> int`) funcionam; um `throw` conta como fim de caminho para o "falta return".
- **Erros de runtime do programa também são capturáveis**, com a mensagem de sempre: divisão por zero, índice fora, chave ausente de map, `@array.pop` vazio, `@into.int("abc")`, `@math.clamp` com limites trocados, `@array.fill` negativo, range grande demais, sleep negativo, `step 0`... Bugs da VM (`TypeMismatch`, `StackUnderflow`, `InvalidFunctionIndex`) NUNCA são capturáveis.
- Sem handler em lugar nenhum: o programa termina com `runtime error: unhandled exception: <msg>` e exit 1 (igual aos outros erros de runtime).

### Como foi feito
- Lexer: `TokenKind::Throw`. Parser: `Stmt::Throw { value, pos }`. Módulos: `annotate`/`rename` ganharam o braço (rename percorre o valor, para nomes importados dentro do `throw` serem renomeados). Checker: `TypedStmt::Throw`, `stmt_always_returns` conta `Throw`. Compilador: `compile_expr(value)` + o `OpCode::Throw` que já existia.
- **VM**: `Throw` agora devolve `Err(RuntimeError::UnhandledException(msg))` e quem trata é `run_chunk`: em `Err`, chama `catch_or_propagate` (`#[cold]`, fora do laço quente): se o erro é capturável (`RuntimeError::catchable_message`) e o frame tem handler, trunca a pilha de operandos até a altura guardada no `PushHandler` (descarta temporários do meio da expressão), empilha a mensagem e pula para o catch; senão devolve o `Err` e o `run_chunk` do chamador tenta nos handlers DELE. Isso funciona porque chamadas de função são recursão nativa. `CallFrame.handlers` agora é `Vec<(catch_target, altura_da_pilha)>`.
- **Bug latente corrigido**: `break`/`continue` saindo de um `try` dentro de um laço pulavam o `PopHandler` e deixavam um handler fantasma (um `throw` posterior cairia num catch já inativo). O compilador agora conta `open_handlers` e cada `LoopContext` guarda `handlers_at_entry`; `break`/`continue` emitem os `PopHandler` que faltam (`disarm_handlers_to_loop`).
- LSP: `throw` na lista de palavras-chave, na gramática (`try|catch|throw`) e um snippet `throw`; dentro de `catch (e)`, `e.` oferece os métodos de string (também no nível de topo: a busca de locais agora varre o código de topo pulando o interior de funções/structs, `hidden_ranges`).

### Desempenho
- `tools/perf/bench.py` (5 rodadas) contra o 0.11.0: TOTAL 11,401 s -> 11,394 s (**1,00x**); por benchmark entre 0,94x e 1,08x (ruído). O `Err` saiu do caminho quente.

### Limites / não feito
- Só `string` é lançável: sem `throw MeuErro { ... }` nem `catch (e: MeuErro)` filtrando por tipo. É a extensão natural (precisa de teste de tipo em runtime + relançar se não casar); não pedida.
- Sem `finally`. Sem stack trace. `try`/`catch` continua sendo statement (não expressão).

### Testes
- Suíte 276 -> **282**: `ok/throw_catch` (entre funções, no meio de expressão, aninhado/relançado, `break`/`continue` saindo de `try`, 5 erros de runtime capturados, `return` em `try`, função `-> int` que só lança), 4 `err/*` (`throw_uncaught`, `throw_in_function_uncaught`, `throw_not_string`, `catch_type`), + 1 `--no-opt`. LSP 39 -> **40**. Sem sabotagens (preferência dele).

## Sessão `@file` e `@sys` (2026-10-08) — CONCLUÍDA (v0.12.0 → v0.13.0)

Os dois namespaces novos (agora `NAMESPACES` tem 8: str, array, map, into, math, time, file, sys). Nomes e escopo foram PROPOSTA minha (ele perguntou "o que deveriam fazer?"); fáceis de renomear.

### `@sys` — o programa e o PROCESSO em que roda
- `@sys.args() -> array<string>`: argumentos do programa. **CLI: `spg prog.spg [opções do spg] -- a b c`**; tudo depois de `--` vai para o programa (inclusive coisas que parecem flags, ex. `-- --time`). Sem `--`, argumento solto continua sendo erro ("unexpected extra argument"); `--` só vale ao executar (não em `check`).
- `@sys.exit(code: int)`: termina na hora com esse código de saída (código fora de i32 vira 1). **NÃO é capturável** por `try`/`catch` (`RuntimeError::Exit`; `catchable_message` devolve `None`). O CLI devolve o código sem imprimir "runtime error"; a saída bufferizada de `print` é descarregada antes.
- `@sys.env(nome, reserva) -> string`: variável de ambiente, ou a reserva se não existir/não for texto válido.
- `@sys.readLine() -> string`: uma linha do stdin SEM `\n`/`\r\n`; no fim da entrada **lança** (`EndOfInput`, capturável: "end of input (nothing left on stdin)"). `@sys.readAll() -> string`: tudo que restar (vazio no fim, sem erro). As duas descarregam o stdout antes de bloquear (um prompt impresso por `print` aparece antes da espera).
- `@sys.platform() -> string` (`std::env::consts::OS`: "windows", "linux", "macos"...), `@sys.cwd() -> string`.
- NÃO existe (ideia, não pedida): executar comandos externos (`@sys.run`), `@sys.time`/data local, `@sys.pid`.

### `@file` — o disco (caminhos são `string`; `/` funciona no Windows também)
- `read(p) -> string`, `write(p, texto)` (cria/sobrescreve), `append(p, texto)` (cria se não existir), `lines(p) -> array<string>` (separa em `\n` e `\r\n`, sem a linha vazia final), `exists(p) -> bool`, `isDir(p) -> bool`, `list(dir) -> array<string>` (NOMES, ordenados, sem prefixo de pasta), `mkdir(p)` (cria também os pais), `delete(p)` (arquivo ou pasta VAZIA; **nunca recursivo**), `copy(de, para)`, `rename(de, para)`, `join(a, b) -> string` (junta caminhos com o separador do sistema).
- Toda falha de I/O é um erro de runtime **capturável** (`RuntimeError::Io`) com texto estável: `@file.read("x.txt"): no such file or directory` (também `permission denied`, `already exists`, `not valid UTF-8 text`, ou a mensagem do SO). Sem `try`: termina com `runtime error: ...` e exit 1.
- **v0.14.0 (fechamento do `@file`)**: `deleteAll(p)` (arquivo OU árvore inteira; nome próprio e explícito para o `delete` continuar seguro; **recusa a raiz do sistema de arquivos**, `canonicalize(p).parent().is_none()`), `size(p) -> int` (bytes; pasta = erro `is a directory`), `modified(p) -> int` (ms desde 1970, MESMA unidade do `@time.unix()`), `modifiedDate(p) -> string` (ISO em UTC, MESMO formato do `@time.now()`, ex. `2026-10-08T22:27:52Z`), `readBytes(p) -> array<int>` (cada item 0..255; acima de 20 milhões de bytes = `RangeTooLarge`) e `writeBytes(p, array<int>)` (valida TODOS os itens ANTES de tocar o disco: item fora de 0..255 = `InvalidByte`, arquivo nenhum é criado/alterado). Não há data de criação (não existe em todo SO).
- NÃO existe: abrir arquivo como handle/stream (tudo é "arquivo inteiro de uma vez"), glob, permissões/atributos, ler/escrever em posição.

### Como foi feito
- Builtins só no caminho genérico da VM (`call_builtin`); nada no caminho quente. Argumentos do programa: `static PROGRAM_ARGS: OnceLock<Vec<String>>` + `spg_vm::set_program_args` (o CLI chama antes de `Vm::new`). `RuntimeError`: `Exit { code }`, `Io { function, path, reason }`, `EndOfInput`.
- Lexer: ganhou a escape **`\r`** em string com aspas (antes qualquer escape desconhecido virava o próprio caractere: `"\r"` era a letra `r`). As outras continuam: `\n`, `\t`, `\"`, `\\`. String crua `'...'` não tem escapes (útil para caminhos do Windows).
- Editor: `@file.` e `@sys.` listam os membros sozinhos (tabela); gramática e README da extensão atualizados.

### Testes
- Suíte 282 -> **288**: pasta nova `examples/multi/sys/` (FORA de `ok/`, porque precisam de argumentos/stdin e não entram na checagem `--no-opt`): `file_ops` (arquivos de verdade num `mktemp -d`, passado por `-- dir`), `exit_code` (exit 7 não capturável + args com espaço e `--time`) e `stdin` (por pipe, com `\r\n`), + 3 `err/*` (`file_read_missing`, `sys_exit_type`, `file_arg_type`). Helper novo `check` no `run.sh`. Sem sabotagens (preferência dele). Não testado: Windows de verdade (o container é Linux; `list`/`join`/`cwd` usam `std::path`).

## `@file` fechado (2026-10-08) — v0.13.0 → v0.14.0
Detalhes na seção anterior (bloco "v0.14.0 (fechamento do `@file`)"). Suíte 288 -> **290** (`sys/file_meta` com `check`, + `err/file_bytes_type`). `deleteAll` na raiz NÃO tem teste automático (de propósito: se a guarda falhasse, o teste apagaria o contêiner); a guarda é só código. Sessão encerrada por ele ("completinho e terminamos"); o que sobra está em "Plano da PRÓXIMA sessão".

## Plano da PRÓXIMA sessão (era: `map<K,V>`, FEITO)

Ordem combinada com o usuário: `enum` (**FEITO em 2026-10-05**), agora **`map<K,V>`**. (`print`/`printError` variádicos já estão feitos.)

### Decidido pelo usuário
- **Enum PERCORRÍVEL**: `for c in Color { ... }` (feito).
- **Pipe `|>`**: só ANOTADO, fica de lado (ele acha que é o que mais dá trabalho; na minha estimativa o desugar `a |> f(b)` -> `f(a, b)` no parser é pequeno, mas a decisão de deixar de lado é dele).
- A sintaxe parecida com JS está OK para ele ("JS é o esqueleto de todas as linguagens"): não precisa forçar sabor próprio nas construções novas.

### NÃO aprovado (não assumir)
- `pick (c) { Red -> ... }` exaustivo para enum.
- `->` no literal de map (`["ana" -> 30]`). O literal do map está EM ABERTO: confirmar com ele antes de chegar no `map`
  (a alternativa convencional é `["ana": 30]`, mapa vazio `[:]`).

### Desenho proposto por mim para o map (ainda não confirmado em detalhe)
- `map<K,V>`: chaves `int`/`string`/`bool` (nunca `float`), ordem de inserção, `m[k]`/`m[k] = v`/`m[k] += v` reaproveitando o índice de array,
  ler chave ausente = erro de runtime claro, `@map.has/get(m, k, reserva)/remove/len/keys/values`, `Value::Map` na VM, `Ty::Map(K,V)`.

### Lacunas do pacote (não são tipos; anotadas, na ordem de custo/valor que eu vejo)
(`throw` FEITO na 0.12.0), (açúcar `xs.push(1)`/`s.len()` FEITO na 0.11.0), (`@array.remove/insert/fill/clear` e `@math.sin/cos/log/exp/tau` FEITOS na 0.9.0),
(`@sys.exit`/argumentos e `@file` FEITOS na 0.13.0), (`@str.padStart/padEnd/lastIndexOf/trimStart/trimEnd` FEITOS na 0.10.0),
interpolação de string (`"${x}"`; `'...'` crua nunca interpolaria; quebra strings existentes com `${`), tipo função (`function(int) -> int`,
sessão própria), `T?` (opcional; o mais caro, mexe em todo operador tipado), tuplas. `char` descartado: `'a'` já é string crua.
Também: métodos estáticos, hover e go-to-definition no LSP, `spg fmt`, install.sh,
`@time.date()`, fuso local, traduzir o `run.sh` para inglês (tem texto em português de antes; só o que eu adicionei está em inglês).

## Convenções do usuário

Nomes expostos ao .spg em camelCase, sem "_". Prefere eficiência e praticidade.

## Armadilhas já pisadas (não repetir)

- Replace textual global (`.display()` → `friendly()`) atingiu o corpo da própria função e
  gerou recursão infinita; reconferir a função-alvo depois de qualquer replace amplo.
- Placeholder esquecido num script de teste apagou checks e deixou tudo verde sem testar
  nada: por isso mudança de teste sempre é provada por sabotagem.
- `{name}` dentro de string literal de `write!` com args posicionais NÃO interpola.
- Dois `cargo build` seguidos enganam ("Finished 0.03s" no 2º); pra forçar, `touch` no arquivo.
- Comparar performance só por ALTERNÂNCIA de binários na mesma janela (a máquina varia).
- Ao escrever uma varredura nova do AST (ex. `annotate.rs`), cuidado com variantes que
  carregam `Vec<T>`/subexpressões (ex. `Expr::ArrayLit`) — fácil agrupar num braço "só
  carimba a Pos" e esquecer de visitar o que está dentro. Pego antes de rodar desta vez,
  mas quase passou; ganhou teste de sabotagem dedicado por isso.
- `mkdir -p {a,b,c}` no `sh` do container NÃO expande chaves (criou uma pasta literal); e
  `[[`, `==` e `$PIPESTATUS` não existem lá: scripts de teste com isso vão em `bash -c`/`#!/bin/bash`.
  Um comparador quebrado por isso acusou "18 diferentes" (falso) — e o oposto também poderia
  acontecer: validar o comparador com uma diferença conhecida.
- `pgrep -f "padrão"` dentro de `sh -c` casa com o próprio shell; usar `[s]pg` no padrão.
- **Binário sabotado fica em `target/`**: o script de sabotagem restaura o código-fonte mas, se o build da sabotagem FALHAR,
  a suíte roda no binário da sabotagem ANTERIOR e o resultado engana. O script agora aborta se o build falhar; e depois de
  qualquer rodada de sabotagem: `touch crates/spg-cli/src/main.rs` + `cargo build --release` antes de qualquer outra coisa.
- Um `sed` com barra invertida + ponto num arquivo PowerShell/regex não casou (escape duplo no shell); para trocar texto com barras e regex use
  Python com string crua (`r"..."`) e `assert s.count(a)==1`.
- `;` NÃO separa statements em .spg (um teste meu quebrou por isso); uma instrução por linha.

## Diretivas de arquivo: `#> friendly-terminal` (sessão 2026-10-08)

- Cabeçalho do arquivo: `#>` só vale ANTES de qualquer código (comentários `//` e linhas em branco podem vir antes). `#>` depois de código = `MisplacedDirective`, erro bruto.
- `#>name` e `#> name` são iguais. A diretiva ocupa a linha inteira (só espaço ou comentário depois).
- Diretiva desconhecida = erro, com sugestão quando a distância de edição é ≤ 2 (`#> friendly-terminl` → "did you mean"). Diretiva repetida = erro.
- Escopo POR PROGRAMA: só o arquivo de entrada pode ter diretivas. Diretiva em módulo importado = `ModuleError::DirectiveInImport` (decisão nossa, a confirmar).
- Implementação: `DIRECTIVES` e `FileSpec` em `spg-lexer`; `Lexer::tokenize_with_spec()` lê o cabeçalho e não emite tokens para ele; `read_file_spec(src)` lê só o cabeçalho (a CLI usa isso para decidir o tom mesmo com o corpo quebrado).
- `ModuleError::Lex` agora guarda o `LexError` estruturado (além da mensagem técnica, que segue igual para o LSP).
- Tom friendly agora em TODOS os erros do engine: `LexError/ParseError/TypeError/CompileError/RuntimeError/ModuleError::friendly()`. Cada um mantém o fato técnico e acrescenta "Oops!" + dica por etapa. As dicas (`HINT_*`) e o formador (`friendly_sentence`) ficam em `spg-lexer`; vm e compiler ganharam essa dependência. A CLI escolhe o tom pelo cabeçalho do arquivo de entrada.
- `ModuleError::Parse` agora guarda o `ParseError` estruturado (mesmo padrão do `Lex`).
- Versão NÃO foi alterada (continua 0.14.0): subir para 0.15.0 exigiria reempacotar `dist/*.vsix` com a mesma versão.
- Testes: 5 unitários novos em `lexer.rs` (`directive_tests`); suíte multi 290 ok; `cargo test --workspace` verde. Checagem manual com `#>` em entrada, em import, typo, duplicata e fora do topo.
- (ATUALIZADO na seção "Pacote de diretivas fechado") `timeout` e `strict` já foram feitos. Ainda só ideia: `#> version`, `#> trace`, sandbox (`no-io`).

### Fechamento do friendly-terminal (2026-10-08, parte 2)

- Estendido a todos os erros: parse, tipos, compilação, runtime e módulos. Testado nos dois tons com um caso de cada etapa.
- Versão segue 0.14.0. Subir para 0.15.0 junto com o reempacotamento do `.vsix`, quando o pacote de diretivas estiver concluído.
- (ATUALIZADO) `#> timeout N` e `#> strict` feitos na parte 3, abaixo.

## Pacote de diretivas fechado (2026-10-09, parte 3): `timeout N` e `strict`

- Três diretivas: `friendly-terminal`, `timeout N`, `strict` (`DIRECTIVES` em `spg-lexer`). `FileSpec { friendly_terminal, strict, timeout_ms, directives }`. `timeout` exige inteiro > 0; as outras NÃO aceitam argumento (`BadDirectiveArgument`). Escopo por programa: só o arquivo de entrada tem diretivas, então `strict` vale para o programa inteiro (arquivos importados incluídos).
- **`#> timeout N`**: watchdog em THREAD (`start_watchdog` na CLI), contado a partir do início da execução (depois de compilar). Por ser thread, corta também `time.sleep` e `readLine` bloqueado. Sai com código **124** e `runtime error: timeout: the program ran for more than N ms` (com friendly: `error: Oops! ...`). Faz `flush_stdout_in_panic` (try_lock): se a VM estiver segurando o lock de saída naquele instante, o último pedaço do buffer pode se perder. Só vale para executar (`spg run`), não para `check`.
- **`#> strict`** só ENDURECE regras; a gramática é a mesma nos dois modos:
  1. Overflow de int vira erro de runtime `IntegerOverflow { op }` (soma, subtração, multiplicação, `MIN / -1`, negação, `abs`, `**`, `@array.sum`), capturável por `try/catch`. Sem strict o int dá WRAP (complemento de dois), agora definido e igual em toda a VM (`crates/spg-vm/src/arith.rs`, flag global `set_strict`, lido só no caminho frio do overflow). `MIN % -1` = 0 nos dois modos. Antes, `MIN / -1` e `MIN % -1` com constante davam panic do Rust; o expoente de `**` truncava para u32 e agora é limitado.
  2. Nome que esconde outro de escopo externo (`ShadowedDeclaration`): vale para `let`, parâmetro, variável de `for` e de `catch`, inclusive contra global. Isentos: nomes que começam com `_`, nomes internos e o `me` implícito de método.
  3. Código depois de `return`/`break`/`continue`/`throw` no mesmo bloco (`UnreachableCode`, um erro por bloco).
  - API: `type_check_with(program, strict)`; `type_check(program)` = não strict (o LSP continua usando este).
- Lembrete de sintaxe: a linguagem não tem terminador de statement; `return` + nova linha + `print(2)` é lido como `return print(2)`.
- Efeito colateral bom: `infer_return_type` agora descarta os erros do passe de sondagem já desde o `declare` dos parâmetros (antes, erro de parâmetro duplicado podia sair em dobro).
- Medição de performance (15M iterações de aritmética + `fib(30)` + 3M de `a[j % 4] += 2`, 14 rodadas alternadas antes/depois): mediana 1989 ms -> 1946 ms. Sem custo mensurável.
- Varredura: `#> strict` sobre os 228 exemplos sem `import`. Só 3 acusaram regra do strict, e são sombras reais de propósito dos testes (`for k in 0..k`, parâmetro `n` contra global, `let me` global); o `me` implícito foi isentado por causa do terceiro.
- Testes: 17 novos em `examples/multi` (prefixo `dir_`; os de timeout exigem a mensagem do PRÓPRIO engine, porque o `timeout 30` da suíte também dá 124 numa trava). Suíte multi: **312 ok, 0 falhas**. Unitários novos: `directive_args_tests` (lexer) e `arith` (VM).
- (RESOLVIDO na seção abaixo, "LSP e extensão conhecem as diretivas") LSP/extensão não conheciam as diretivas. Versão segue 0.14.0.
- Candidatos futuros para o strict (NÃO feitos, com cuidado anotado): variável/parâmetro não usado (`infer_return_type` declara parâmetros sem marcar uso, daria falso positivo); expressão sem efeito como statement (o último `ExprStmt` de um bloco é o valor do `if` como expressão, então não dá para proibir às cegas).

## LSP e extensão conhecem as diretivas (2026-10-09, parte 4)

- **Diagnósticos** (`spg-lsp/src/diag.rs`): `analyze` lê o cabeçalho `#>` do arquivo aberto (o buffer vale mais que o disco) e chama `type_check_with(program, strict)`. Os erros do `#> strict` (nome escondido, código morto) aparecem como sublinhado no editor, e diretiva desconhecida/repetida/fora do topo já aparecia pelo resolver. Estouro de int e timeout são de runtime: não aparecem no editor.
- **Completion** (`server.rs`, `directive_completion`): depois de `#>` oferece `friendly-terminal`, `timeout` (insere `timeout ` com espaço) e `strict`, sem repetir as que o arquivo já usa. Cada item leva `textEdit` com o range do nome parcial, por causa do `-` de `friendly-terminal`. Com argumento ou comentário sendo digitado, devolve lista VAZIA (não cai na lista global). `DIRECTIVE_DOCS` é a tabela de ajuda; um teste exige uma entrada para cada nome de `spg_lexer::DIRECTIVES`. Sem caractere de gatilho novo (um `>` dispararia a lista global em todo operador).
- **Gramática** (`editors/vscode/syntaxes/spg.tmLanguage.json`): padrão `directives` antes dos comentários (`meta.directive.spg`; `#>` = `punctuation.definition.directive`, nome = `keyword.other.directive`, número = `constant.numeric.integer`). Validado com `vscode-textmate` de verdade: comentário no fim da linha segue comentário e `"#> x"` dentro de string não é afetado.
- `editors/vscode/README.md` ganhou a seção "File directives". `extension.js` NÃO mudou (já repassava `textEdit`/`range`).
- Testes: 1 em `diag.rs` (`strict_header_turns_on_the_strict_rules`) e 2 em `server.rs`; spg-lsp = 43 passando. `tools/ext-tests` NÃO foi rodado nesta sessão: o `vscode` falso fica em `tools/ext-tests/node_modules`, que não veio no zip (rodar localmente).
- (FEITO na seção abaixo) subir para 0.15.0 e reempacotar o `.vsix`.

## 0.15.0 fechada (2026-10-09, parte 5)

- Versão 0.14.0 -> **0.15.0** em `Cargo.toml` (workspace; `Cargo.lock` regenerado pelo build), `editors/vscode/package.json`, `README.md` e no cabeçalho deste arquivo. `dist/spaghetti-0.14.0.vsix` REMOVIDO e substituído por `dist/spaghetti-0.15.0.vsix` (o instalador compara a versão da extensão: o `.vsix` novo precisava de versão nova para não ser tratado como "já instalado").
- `.vsix` empacotado com `vsce package --no-dependencies --allow-missing-repository --skip-license` (vsce instalado em /tmp). Conteúdo conferido: `package.json` 0.15.0, gramática com `directives`, `extension.js` idêntico ao do repositório. O `.vsix` NÃO embute binário (o `spg.exe` é instalado à parte).
- `install.ps1` não precisou mudar (a única menção a 0.14.0 lá é um comentário de exemplo).
- NÃO rodado nesta sessão (precisa do Windows/VSCode do usuário): `tools/installer-tests/run.ps1`, a instalação real do `.vsix` e `tools/ext-tests/run.sh` (o `vscode` falso fica em `tools/ext-tests/node_modules`, que não veio no zip).

## Decisão: SEM `#> version` (2026-10-09, parte 6)

- O dono decidiu NÃO fazer `#> version` (nem como leitor, nem como modificador). Compatibilidade entre versões da linguagem é resolvida na DISTRIBUIÇÃO: um download por versão (v0.14, v0.15, ...), e cada arquivo roda no motor da versão para a qual foi escrito. O motor nunca carrega código de versões antigas (sem pasta `versions/`, sem portas `Feature`).
- Pacote de diretivas final: `friendly-terminal`, `timeout N`, `strict`. Não reabrir sem pedido dele.
- Consequência para o `strict`: ele pode ficar mais rígido em qualquer versão (como um linter), já que cada versão tem o seu próprio download.
- Princípio dele para o `friendly-terminal`: só muda o TOM, nunca a informação (o terminal técnico não pode informar menos que o amigável). Ponto em aberto, a decidir com ele: hoje o amigável acrescenta uma dica genérica por etapa (`HINT_*`) que o técnico não mostra; opção recomendada = o técnico passar a mostrar a mesma dica numa linha `hint:`.
- Decisão dele: widening int→float NÃO entra (int e float ficam separados, para evitar operadores extras tipo `/` e `//`). Pendência restante da lista: métodos estáticos.

## `@json` (2026-10-10, parte 7) — lançado na 0.16.0

- Namespace novo `json` (`NAMESPACES` agora tem 9). 9 funções: `@json.stringify(v)` e `@json.pretty(v)` (qualquer valor -> texto); `get`, `has`, `type`, `len`, `keys`, `valid` (consultam o TEXTO por caminho); `@json.set(texto, caminho, valor)` (valor qualquer -> texto novo).
- Decisão de desenho (dele: "seguimos"): LER devolve só strings/bool/int, sem tipo dinâmico `json`, porque a linguagem é tipada e os arrays/maps são homogêneos. Decodificação tipada (`let c: Config = @json.parse(texto)`) ficou para depois, só se sentir falta. Nome `@json` (e não `@format`): `@format.json()` não diz a direção, e lembra `printf`; outros formatos entram como namespaces irmãos (`@toml`, `@csv`) com as mesmas funções.
- Implementação: `crates/spg-vm/src/json.rs` (parser RFC 8259 e serializador à mão, sem crate externo; profundidade máx 512), braços em `builtins.rs` (caminho genérico `call_builtin`), variantes `Json*` em `BuiltinId`, assinaturas em `spg-typecheck/src/builtins.rs` (`Ty::Any` nos parâmetros que aceitam qualquer valor), `RuntimeError::Json { function, message }` (capturável por `try/catch`).
- Caminhos: `a.b.0.c`; segmento numérico indexa array (em objeto é só nome de chave); `\.` e `\\` escapam; `""` = raiz.
- `get`: string decodificada; número/bool como texto (número como foi escrito); `null` = `"null"`; objeto/array = JSON compacto (dá para consultar de novo). Caminho inexistente = erro com o ponto onde parou (`has` testa antes); texto inválido = erro com linha e coluna.
- `set`: só o ÚLTIMO passo pode ser novo (chave nova; índice == tamanho adiciona no fim). Reescreve o documento NO ESTILO DE ENTRADA: compacto se estava numa linha; senão uma linha por item com a mesma indentação (tab ou N espaços); mantém `\n` final e `\r\n`. CUIDADO: objeto/array que estava inline numa linha é expandido, e `\uXXXX` vira UTF-8. Números ficam como escritos.
- `stringify`/`pretty`/`set` (valor -> JSON): struct vira objeto (nomes dos campos), enum vira o nome da variante, chave de map vira texto, float usa `{:?}` (`1.0` continua float), NaN/infinito e `void` = erro, valor que contém a si mesmo = erro (profundidade). `@json.pretty("texto")` devolve uma STRING JSON (com aspas), NÃO reformata um texto JSON.
- Testes: 10 unitários em `json.rs`; 6 em `examples/multi` (`json_*`); suíte multi 321 ok / 0 falhas; o teste antigo `err/ns_desconhecido` foi atualizado (a lista de namespaces agora termina em `@json`). Fluxo real testado: ler `package.json`, iterar chaves, editar versão, gravar e reler.
- Gramática da extensão: regex de namespaces inclui `json`. LSP: a lista de namespaces vem de `NAMESPACES` (só o teste foi atualizado).
- (FEITO na 0.16.0) versão e `.vsix` reempacotados. Ideias futuras: `@json.parse` tipado, reformatar texto JSON, `@toml`/`@csv`.

## 0.16.0 fechada (2026-10-10, parte 8)

- Versão 0.15.0 -> **0.16.0** em `Cargo.toml` (workspace; `Cargo.lock` regenerado pelo build), `editors/vscode/package.json`, `README.md` e no cabeçalho deste arquivo. `dist/spaghetti-0.15.0.vsix` REMOVIDO e substituído por `dist/spaghetti-0.16.0.vsix` (a 0.15.0 continua com o dono, em cópia própria, para quem precisar do motor daquela versão).
- Conteúdo novo desta versão: só o `@json` (gramática com `json` nos namespaces + 9 funções no motor). Nada de diretivas novas.
- `.vsix` empacotado como na 0.15.0 (`vsce package --no-dependencies --allow-missing-repository --skip-license`); conteúdo conferido contra os arquivos do repositório. Não embute binário.
- NÃO rodado nesta sessão (precisa do Windows/VSCode do dono): `tools/installer-tests/run.ps1`, a instalação real do `.vsix` e `tools/ext-tests/run.sh`.
- Decisões em aberto que NÃO foram feitas: dica do `friendly-terminal` também no terminal técnico (princípio dele: só o tom muda); métodos estáticos (descartados por custo/uso); `#> version` (descartado: um download por versão).

## `@map.keysOf` (0.17.0)

- Busca reversa: `@map.keysOf(m, valor)` devolve `array<K>` com TODAS as chaves cujo valor é igual a `valor` (na ordem de inserção). Sem achado = array vazio (não é erro). Empate = várias chaves, sem "último ganha".
- Assinatura no checker: segundo argumento tem o tipo do VALOR do map (`check_map_call`, mesmo `want` do `@map.get`). Erro de tipo: `'@map.keysOf' argument #2 expects int, got string`.
- Igualdade: `Value::structurally_equal` (mesma regra de `@array.contains`/`indexOf`). Como é O(n) como o `contains`, não há índice reverso.
- Aparece sozinho no autocomplete (`@map.` agora lista 7; `m.` também, via tabela de builtins).
- Testes: `ok/map_keysof.spg` (inclui empate, chave bool/int, vazio, forma `m.keysOf(v)`), `err/map_keysof_tipo.spg`; LSP `at_map_dot` 6 -> 7 e lista de `m.` atualizada. Suíte 324 ok.
- Empacotamento: `dist/spaghetti-0.17.0.vsix` (o 0.16.0 foi removido do dist). A extensão não mudou além da versão (a lógica de autocomplete vem do LSP do binário).

## Métodos em todos os namespaces (2026-10-10) — CONCLUÍDA (v0.17.4 → v0.18.0)

### O que existe agora
- `x.fn(args)` vale para toda função de `@str`, `@array`, `@map`, `@math` e `@into` cujo PRIMEIRO parâmetro aceita o tipo de `x`. Ex.: `n.abs()` = `@math.abs(n)`, `f.sqrt()` = `@math.sqrt(f)`, `3.min(9)` = `@math.min(3, 9)`, `2.5.max(9.5)` (overload float), `y.atan2(x)`, `v.clamp(0, 10)`, `f.round()`, `n.sign()`.
- Overload por tipo continua valendo: `min`/`max`/`abs`/`sign`/`clamp` escolhem o overload int ou float pelo receptor.
- Sem coerção: `5.sqrt()` é erro (sqrt aceita só float); use `5.0.sqrt()` ou `n.float().sqrt()`.
- Exclusões (`NOT_METHOD_BUILTINS` em `builtins.rs`): `@math.pi`, `@math.e`, `@math.tau`, `@math.random` (constantes/fonte, não têm receptor) e `@array.range`, `@array.fill` (o primeiro argumento é contagem/limite, não receptor). `5.pi()` e `5.range(10)` dão erro "no method".
- `@file`, `@sys`, `@time` e `@json` NÃO entram (decisão dele: não são de valor).
- Atalhos `x.string()`, `x.int()`, `x.float()` seguem como antes (checados antes do lookup geral; método de struct do mesmo nome ainda vence).
- Pegadinha: `-3.sign()` é `-(3.sign())`, porque o `-` unário se aplica DEPOIS do método (igual ao Rust). Para negativo use `(-3).sign()`.

### Como foi feito (para não refazer)
- Uma única fonte da verdade: a tabela de builtins. `method_namespace(ty, nome)` (em `spg-typecheck/src/builtins.rs`) acha o namespace cuja função `nome` aceita `ty` primeiro (ordem `str, array, map, math, into`). `methods_of(ty)` lista os métodos de um tipo (usado pelo LSP).
- `checker.rs::check_method_call`: o `match` fixo de String/Array/Map virou esse lookup. Os tipos sem namespace (bool, struct, enum, void) seguem pelos caminhos antigos. `ty_matches` virou `pub(crate)`.
- O despacho reaproveita `check_native_method` (que já existia): o receptor é o argumento #0 e as checagens de overload/aridade são as mesmas da chamada normal.
- Erro `NoSuchNativeMethod` ganhou mensagem para int/float (aponta `@math` e `@into`, com exemplo `x.abs() is @math.abs(x)`).
- LSP: `members_of` para `int`/`float`/`bool` usa `methods_of`, então `n.` lista abs, min, max, sqrt..., e `f.` lista int, string.... String/array/map continuam pela lista antiga (a lista de map é comparada exatamente nos testes).

### Testes
- Suíte multi: 331 → 333 ok (1 expectativa atualizada: `err/method_on_int.spg`, que fixava a mensagem antiga "has no methods"; 1 teste novo: `ok/methods_numeric.spg`, cobre int/float/overloads/string/array/join).
- LSP: 43 ok (2 testes atualizados: "um int não tem membros" virou "um int oferece @math"; eram o comportamento antigo).
- Erros provados à mão: `5.pi()`, `5.range(10)`, `5.fill(0)`, `5.nope()`, `true.abs()`, `2.5.repeat(2)`, `5.sqrt(1)`, `5.string(1)`.
- Não rodada: a suíte completa do `cargo test --workspace` (o pedido foi não gastar esforço nos ~400 testes já verdes). Rodados: suíte multi inteira (331 → 333) e `cargo test -p spg-lsp`.

### Ideias NÃO feitas (não assumir)
- Mensagem para `5.sqrt()`: hoje diz "no method 'sqrt'", mas sqrt existe para float; dá para dizer "sqrt takes a float, use 5.0.sqrt()" se ele quiser.

## readLine (2026-10-10) — CONCLUÍDA (v0.18.0 → v0.19.0)

- `readLine()` continua igual. Novo: `readLine(prompt)` escreve o prompt SEM quebra de linha, descarrega a saída e espera a digitação (o texto digitado cai logo após o prompt). Implementado como sobrecarga com id próprio, `BuiltinId::ReadLinePrompt` (typecheck), e `write_out_text` (mesmo buffer do `print`).
- Fim de entrada: `readLine()` e `readLine(prompt)` lançam `EndOfInput`, capturável por `try`/`catch`, com a mensagem `readLine: end of input (nothing left on stdin)`. Antes devolviam `""` silenciosamente (um `while` podia girar para sempre). Quando NÃO há mais entrada, o erro é o único sinal; linha vazia (só Enter) continua devolvendo `""`. Mudança de comportamento: programas que dependiam do `""` no fim da entrada passam a dar erro (decisão assumida).
- `EndOfInput` virou `EndOfInput { function }`, para a mensagem dizer quem falhou (`readLine` ou `@sys.readLine`).
- Erro de I/O do stdin usava `InvalidNumberFormat` (mensagem de número, sem relação); agora `io_error`.
- Um helper só, `read_stdin_line(function)`, serve ao `readLine` e ao `@sys.readLine` (antes eram dois trechos quase iguais, com trims diferentes). O trim agora é o do `@sys` (tira todos os `\r`/`\n` do fim).

### Testes
- Suíte multi 333 -> **334**: novo `sys/readline_prompt.spg` (prompt, `.int()` depois do prompt e EOF capturado com `try`).
- Provado à mão: `readLine("Nome: ")` com entrada; linha vazia; CRLF; fim de entrada sem `try` (`runtime error: readLine: end of input ...`, exit 1); `@sys.readLine` sem mudança.


## Interpolação de string `"${expr}"` (2026-10-10) — CONCLUÍDA (sem bump de versão; a versão sai no release)

### O que existe
- `"oi ${nome}, tem ${idade + 1} anos"`: qualquer expressão dentro de `${...}`. Cada pedaço vira `@into.string(expr)` e a string inteira vira uma cadeia `+` entre parênteses. Sem tokens novos no parser, sem mudança na VM nem no typechecker: é açúcar feito no lexer.
- `'...'` (crua) NUNCA interpola. `\${` vira `${` literal (o escape `\$` já existia como "qualquer caractere escapado"). `$` sem `{` seguinte é texto normal.
- Aninhamento: `"${"x${y}"}"` e `"${@str.len("}")}"`: a leitura do `${...}` pula strings internas (normais e cruas) e `{}` aninhados, então o `}` certo é o que fecha.
- Posições reais: as expressões são tokenizadas com linha/coluna do próprio fonte (`fragment_tokens`), então `undefined variable 'b'` aponta para a variável dentro da string. Os tokens sintéticos (`(`, `+`, `@into.string(`, `)`) herdam a posição do início da string.

### Decisão (assumida)
- Mudança de comportamento: strings que tinham `${` literal passam a interpolar. Para manter literal, escapar com `\${`. Não há mais "dollar sem escape" no caminho normal.

### Erros
- `LexError::EmptyInterpolation` (`"${}"` ou só espaços): "empty interpolation in string, started at line L, column C" / friendly "Oops! Empty `${}` ...".
- String sem fechar dentro de `${` aponta para o início da string externa (`UnterminatedString`).

### Testes
- Suíte multi 334 -> 338: `ok/interpolacao.spg` (expressões, aninhamento, `${}` com `}` dentro de string, `\${`, `$5`, tipos int/float/bool/array, `"${nome}".len()`), `err/interp_vazia.spg`, `err/interp_tipo.spg`.
- `cargo test --workspace` verde (78 unitários). LSP sem mudança de código.

### Não feito / pendente
- Gramática do VSCode (`syntaxes/`) ainda pinta `${...}` como texto puro de string. Ajuste cosmético, fica para o próximo pacote da extensão.
- Autocomplete do LSP dentro de `${` ainda não oferece nomes (o LSP lê os tokens do texto da string).

## Stack trace de erro de runtime (2026-10-10) — CONCLUÍDA (sem bump de versão; a versão sai no release)

### O que o usuário vê
```
runtime error: division by zero
  at divide (main.spg:2)
  at compute (main.spg:6)
  at <main> (main.spg:11)
```
Modo friendly: mesma linha "error: Oops! ..." e depois as mesmas linhas `at`. Módulo importado mostra o arquivo dele (`util.spg:3`). Frames iguais seguidos viram uma linha + `... same line repeated N more times`. Limite de 50 frames no trace; o resto vira `... N more frames`.

### Como foi feito
- **Localização por instrução**: `Chunk` ganhou `locs: Vec<Loc>` (paralelo a `code`) e `cur_loc`. `Loc { file: Option<Rc<str>>, line: u32 }`. `emit` grava `cur_loc`. Custa só memória e tempo de compilação: a VM não lê `locs` no caminho normal.
- **Typed AST**: `SrcLoc { file, line }` (spg-typecheck). `TypedBlock.locs` e `TypedProgram.locs`, um por statement, preenchidos no checker (`stmt_loc`). Granularidade = statement (a linha onde o statement começa). Expressões multi-linha apontam para a linha do statement.
- **AST**: `Stmt::pos()` e `Expr::pos()` (ast.rs). `ExprStmt` usa a posição da expressão.
- **Compilador**: `compile_block` e o loop de `compile_with` setam `cur_loc` antes de cada statement. `to_loc` converte. O peephole (`fuse_pass`) mantém `locs` alinhado: uma instrução fundida usa a localização da primeira.
- **VM**: `run_chunk` registra o frame (`record_frame`) quando o erro sai dele sem ser capturado. A linha vem de `locs[ip-1]` (o `ip` já avançou). Se um `try` captura, o trace acumulado é limpo. `Vm::take_trace()` devolve o trace. `TraceFrame` é exportado.
- **CLI**: `print_trace` depois da mensagem de erro (técnico e friendly).

### Limite de profundidade (mudança junto, para o trace de recursão existir)
- Antes, recursão infinita abortava o processo (`thread has overflowed its stack`, exit 134) sem trace. Agora `MAX_CALL_DEPTH = 100_000` (vm.rs) gera `RuntimeError::StackOverflow { limit }`, **capturável** por `try`/`catch`. A mensagem: "stack overflow: more than 100000 nested calls (is a function calling itself forever?)".
- O limite é conservador: o teste mostrou ~200k frames na thread de 1 GiB do CLI; 100k deixa margem. Se algum dia um frame ficar maior (mais código dentro de `step`), medir de novo com `d(n)` recursivo.
- Contador `depth` incrementado/decrementado dentro do próprio `run_chunk`. Versão com wrapper separado custou ~17% no `fib_bench` (call-heavy); achatada, voltou ao ruído (279 vs 279 ms; call_1arg 1068 vs 1073; fact 2433 vs 2480, ruído).

### Testes
- Suíte multi 338 → 343 ok (sem falhas). Novos: `err/trace_cadeia.spg` (3 frames), `err/trace_recursao.spg` (colapso + contagem de frames omitidos), `err/trace_modulo.spg` + `err/trace_modulo_lib.spg` (arquivo de outro módulo, friendly), `ok/trace_catch.spg` (erro profundo capturado não deixa trace). Expectativas exatas de `print_before_error` e `range_step_zero_dynamic` ganharam as linhas `at` (mudança intencional).
- `cargo test --workspace` verde (78 unitários). Sabotagem não feita, por pedido do usuário (testar o suficiente).

### Limitações / não feito
- Granularidade de statement, não de expressão: em `let x = a[1] + b[9]` numa linha só não há como distinguir, o que já é o padrão da maioria das linguagens.
- Nome da função de método de struct aparece como o nome da função compilada (sem o nome do struct).
- Sem coluna no trace (só linha); o `Pos` tem coluna, mas não foi levada para `Loc`.
- Trace de erro vindo de builtin aparece com a linha do call site (o builtin não tem frame próprio).

## `spg build` (2026-10-10) — CONCLUÍDA (sem bump de versão)

### Decisão de desenho
- O executável é **a própria `spg` + os fontes do programa anexados**, não bytecode serializado. No startup o CLI recompila os fontes embutidos (milissegundos). Vantagens: zero serialização de OpCode/Value/StructDef (o formato mudaria a cada opcode novo), comportamento idêntico ao `spg prog.spg`. Custo: o binário compila de novo a cada execução; se isso doer, o próximo passo é serializar o bytecode (ver nota abaixo).
- Formato (`crates/spg-cli/src/bundle.rs`): payload = `u32` nº de arquivos, depois por arquivo `u32 len + caminho`, `u32 len + fonte` (entrada primeiro); trailer = `u64` tamanho do payload (LE) + `b"SPGBUNDL"`. `read_own` lê só o fim do próprio executável (seek), então um `spg` comum paga um open+seek.
- Entrada: se o executável tem trailer, `main` ignora os subcomandos e roda o programa embutido; **todos** os argumentos vão para `@sys.args()` (sem `--`).

### Resolver (spg-modules)
- `resolve_collect(entry, overlay)` devolve o programa e a lista `(caminho, fonte)` de cada arquivo lido. `resolve_with` virou um wrapper.
- `canonicalize_in`: caminho presente no overlay é aceito **lexicamente** (sem tocar o disco). Isso permite rodar a partir dos fontes embutidos mesmo com os caminhos originais inexistentes na máquina do usuário. Imports relativos passam pelo mesmo caminho (`load_dep`).
- Efeito colateral aceito: o LSP também passa a aceitar overlay de arquivo novo não salvo.

### CLI
- `spg build <arquivo.spg> [-o NOME]`: valida exatamente como `check` (erro de tipo/compilação = não gera nada, exit 1), coleta os fontes, copia o próprio executável e anexa o payload. Default: `<stem>` no Linux, `<stem>.exe` no Windows. Permissão 755 no Unix.
- Refatoração: `compile_resolved` (tipos + compilação de um `Program` já resolvido), `execute_program` (dump, args, timeout, erro+trace) compartilhados entre `run`/`check`/bundle. `run_bundle` é o caminho do executável gerado.
- Mensagem de erro de uso: `'-o' needs a name for the executable`, `missing input file`.

### Testes
- Suíte multi 343 -> 344: `build/interpolacao` (build, roda de dentro de `/` e compara a saída). Verificado à mão: executável rodando depois de apagar os `.spg` (imports incluídos), erro de tipo bloqueando o build, trace e friendly dentro do executável, args.
- `cargo test --workspace` verde (78).

### Limitações / próximos passos
- Caminhos de arquivo no trace/erro de um executável são os caminhos **do momento do build** (o que o usuário digitou). Não são sensíveis hoje, mas vazam diretório de quem compilou. Ajustar se virar problema.
- Não há bytecode serializado (decidido de propósito); só vira prioridade se o tempo de startup incomodar em programas grandes.
- Ainda não há bundle para Windows testado de fato (o container é Linux); o código usa só `std`, e o `.exe` é gerado por `cfg!(windows)`.

## Tipo função, lambdas e callbacks (2026-10-11) — CONCLUÍDA (sem bump de versão)

### Sintaxe
- Tipo: `function(int, string) -> bool` (o `-> R` é obrigatório; `-> void` para nada). Aceita-se em qualquer lugar onde se escreve tipo (parâmetro, retorno, variável, campo, `array<function(int) -> int>`).
- Lambda (expressão): `function(x: int, y: int) -> int { return x + y }`. Parâmetros com tipo obrigatório, como em qualquer assinatura. Sem `-> T`, o retorno é `void`.
- Função nomeada como valor: `let f = dobro`, `aplica(dobro, 3)`. Sem chamada, o nome vira um fechamento sem capturas (`MakeClosure { ncap: 0 }`).
- Chamada de valor: `f(3)` quando `f` é variável/parâmetro de tipo função; `makeAdder(2)(3)`; `function() { ... }()` (IIFE). Se o callee não é nome nem método, é avaliado como expressão.
- No começo de statement, `function (` é expressão (IIFE), não declaração; `function nome(` continua declaração.

### Semântica de captura (decisão de design, leia antes de usar)
- **Captura por valor, no instante em que o lambda é criado.** Um lambda que lê uma variável local de fora recebe uma CÓPIA. `let total = 10; let g = function() -> int { return total }; total = 99; g()` devolve **10**. (Structs e arrays são ponteiros: a mudança interna aparece, a troca do nome não.)
- **Atribuir a uma variável capturada é erro de tipo** (`CaptureAssign`): "a lambda cannot assign 'x': it is a copy of a variable from the function it was written in". Dentro do próprio lambda, variáveis novas e atribuições a elas são normais.
- **Globais (`let` do topo) NÃO são capturadas**: são lidas e escritas direto na slot global, então o lambda vê e altera o valor atual.
- Captura em vários níveis: se um lambda interno usa uma local do avô, o lambda do meio também captura (para poder passar adiante). Feito em `Checker::resolve_name` (um frame por lambda ativo; `base` = escopo onde ele começa).
- Alternativa futura, NÃO feita: captura por referência (caixas compartilhadas). Só vale se houver uso real que exija mutação de variável de fora dentro de lambda.

### Implementação
- **Tipos**: `Ty::Function { params, ret }` (typed_ast), `TypeExpr::Function` (ast). Igualdade estrutural; `ty_matches` já cobre por `==`.
- **Checker**: `resolve_name` (leitura com captura; usado em `Expr::Ident` e na chamada por nome), `assigns_capture` (guarda do Assign), `check_value_call` (aridade e tipos contra o tipo função), braço `Expr::Lambda` (novo frame, `current_return` próprio, `check_all_paths_return` com nome `<lambda>`). Chamada por nome: variável ganha de builtin/função do mesmo nome (sombreamento normal).
- **Compilador**: lambdas viram chunks num `LambdaSink` compartilhado (`Rc<RefCell<..>>`). O índice da função é reservado ANTES de compilar o corpo: `base` (nº de funções nomeadas) + posição. Ao final de `compile_with`, os lambdas são anexados a `functions` (e `function_names` ganha `<lambda>`), antes do peephole. Os capturados viram os primeiros locais do chunk, depois os parâmetros.
- **VM**: `Value::Func(Rc<Closure { index, captures }>)`. `MakeClosure { func_index, ncap }` consome os capturados da pilha. `CallValue { argc }`: pilha = [callee, args...]; os capturados vão antes dos argumentos em `locals`, e a chamada reusa `run_chunk` (trace, `try`/`catch`, limite de profundidade valem igual).
- **Módulos**: `rename.rs` trata lambda como função (parâmetros sombreiam nomes de módulo; o corpo é renomeado com a mesma regra). `annotate.rs` carimba posições. Testado: função importada passada como valor e usada dentro de lambda.
- Exibição: `print(f)` mostra `<function>`; JSON recusa função (`a function has no JSON form`). Tipo no erro: `function(int) -> int`.

### Testes
- Suíte multi 344 -> 352: `ok/lambdas.spg` (somador que retorna lambda, função de valor, IIFE, captura de global e de local), `ok/lambda_captura.spg` (captura em 3 níveis; captura por valor), `err/lambda_captura_atribui.spg`, `err/lambda_tipo_argumento.spg`, `err/lambda_sem_retorno.spg`, `err/trace_lambda.spg` (trace com `<lambda>`). Mensagem de não-chamável atualizada: `only functions can be called, and this value is not a function`.
- Desempenho (alternando binários): fib 278/277 → 253/257 ms; call_1arg 1054 → 1013 ms. Sem regressão.
- `cargo test --workspace` verde (78).

### Não feito / pendente
- LSP: autocompletar não sugere parâmetros de lambda dentro do corpo nem o tipo de variável função em `f.`. Não tem `f.` (função não tem método).
- Documentação da linguagem (página/referência) ainda não existe; quando existir, entra lá o aviso sobre captura por valor.
- `function` como tipo de struct field e método: funciona como campo comum; não há sintaxe especial.

## LSP: funções de valor, parâmetros e assinatura (2026-10-11) — CONCLUÍDA (sem bump de versão)

### O que mudou para o editor
- **Parâmetros no autocomplete**: os parâmetros de toda função nomeada e de todo lambda do arquivo aparecem como completion (kind 6), com `parameter: <tipo>` no detalhe. Antes, parâmetro não aparecia em lugar nenhum. Vale no arquivo inteiro, como os outros nomes da lista (heurística de tokens, de propósito).
- **Detalhe com assinatura**: funções nomeadas, variáveis de tipo função e lambdas ligados a `let` mostram a assinatura no detalhe, por exemplo `mult(a: int, b: int) -> int`.
- **`textDocument/signatureHelp`** (novo, anunciado com gatilhos `(` e `,`): ao digitar `f(` ou depois de cada vírgula, mostra a assinatura e destaca o argumento atual. Funciona para funções nomeadas, valores de tipo função (variável, parâmetro, campo) e lambdas ligados a `let`.

### Como foi feito
- Novo módulo `crates/spg-lsp/src/signature.rs`, só com tokens (como o resto do LSP). Lê tipos (`function(..) -> R`, `array<T>`, `map<K, V>`), parâmetros e a chamada em volta do cursor.
- `call_at` anda para trás contando parênteses e vírgulas. Um bloco `{ ... }` balanceado dentro da chamada (um lambda passado como argumento) é pulado inteiro; só um `{` não casado encerra a busca, para não vazar para o bloco vizinho.
- `server.rs`: capacidade `signatureHelpProvider`, handler `textDocument/signatureHelp` (usa `member_context` para tokens e posição), e detalhes/parâmetros no `completion`.

### Bugs achados no caminho (já corrigidos)
- Busca de chamada parava no `}` do lambda passado como argumento: `aplica(function(..) {..}, 4)` não mostrava assinatura no argumento 1. Corrigido com o pulo de blocos.
- Leitura de tipos: o `>` de `->` era contado como fechamento de `<...>`. Corrigido nos dois scanners (`from_function_type`, `split_top`).

### Testes
- Unitários do módulo (7): rótulo de função nomeada, parâmetro de tipo função, lambda ligado a `let`, índice do argumento com vírgulas aninhadas, parâmetros de lambda no completion, e chamada que não vaza para fora de bloco. Workspace: 84 ok (eram 78).
- Ponta a ponta pelo protocolo (`spg lsp`, stdio): `mult(` arg 0 e 1; `aplica(` com lambda como argumento 0 e arg 1 depois do lambda; completion dentro do corpo do lambda com `valor`, `a`, `b`, `s` etc.
- Suíte multi: 352 ok (sem mudança no compilador nesta etapa).

### Ainda NÃO feito (LSP)
- Hover (tipo/assinatura ao passar o mouse) e go-to-definition: itens da lista de ferramentas, não deste lote.
- Signature help dentro de chamada de método (`p.f(`), que segue o caminho de `members.rs` e não foi estendido.
- Sugestão de tipo ao escrever `function(` em posição de tipo.

## LSP fechado: hover, go-to-definition, assinatura de método, `.string()` (2026-10-11) — CONCLUÍDA (sem bump de versão)

### O que o editor faz agora
- **Hover** (`textDocument/hover`, markdown com bloco `spg`): função e método mostram a assinatura (`dobro(x: int) -> int`); `let` mostra o tipo (anotado ou inferido de literal, struct literal, lambda ou função); parâmetro, campo (`field Ponto.x: int`), struct e enum.
- **Go-to-definition** (`textDocument/definition`): variável ou parâmetro vai para a declaração mais próxima antes do cursor; função, struct e enum vão para a declaração do arquivo; nome importado (`import { x } from "f.spg"`) abre o arquivo de origem na declaração. Campo e método após ponto resolvem pelo tipo do receptor (`p.soma`, `me.x`).
- **Assinatura de método** (`textDocument/signatureHelp` em `p.f(`): mostra a assinatura do método do struct do receptor e destaca o argumento. Tem prioridade sobre uma função livre de mesmo nome.
- **Autocomplete**: `.string()` em todo valor (inclusive struct, enum, bool, e nos namespaces nativos); `.int()` e `.float()` em números. Era o item pendente desde a 0.17.4.

### Como foi feito
- Módulo novo `crates/spg-lsp/src/navigate.rs`: `declarations` (lista tudo que um nome pode ser, com tipo, posição, dono e assinatura), `word_at` (identificador sob o cursor), `target` (regra de resolução: `let`/parâmetro antes do cursor, depois global, depois import), `import_of`, `describe` (texto do hover), `method_signature`.
- `signature.rs`: `read_params` e `return_after` públicos; `call_site` devolve o índice do callee (base de `call_at`, que continua igual para quem já usava).
- `members.rs`: `members_of` virou um wrapper que acrescenta os atalhos de conversão sobre `members_of_own`.
- `server.rs`: capacidades `hoverProvider` e `definitionProvider`; handlers `hover`, `definition`, `resolve_at`, `locate`; `signature_help` tenta método antes de função livre. Caminhos de import são normalizados (`./x.spg` sem o `.` no meio da URI).

### Bugs achados no caminho (corrigidos)
- `import_of` parava no `}` e nunca achava o `from` (o `from` vem depois da chave).
- Tipo de lambda usava os nomes dos parâmetros em vez dos tipos (`function(x) -> int`).
- URI de arquivo importado saía com `/./`.

### Testes
- Unitários: `navigate` (5: campos e métodos com dono, tipo de `let`, local ganha de global, import, chamada de método pelo tipo do receptor). Workspace: 89 ok.
- Atualizados por mudança de comportamento (listas exatas de membros agora incluem `string`): `members` (5 asserções) e `server` (1 asserção).
- Ponta a ponta pelo protocolo com `spg lsp`: hover de `soma`, `dobro` (importada), `let p: Ponto`; definition de `f`, `soma` (campo do struct), `dobro` (arquivo de origem); signatureHelp de `soma(` e de `f(` (lambda ligado a `let`).
- Suíte multi: 352 ok.

### Limitações conhecidas (LSP)
- Tipo de variável sem anotação e sem valor óbvio (ex.: `let x = f(2)`) aparece sem tipo no hover. Resolver isso exige o tipo real do checker, com posições; fica como melhoria futura.
- Colunas são as do lexer (caracteres). Para o hover e a definição, a conversão para UTF-16 é feita só na definição; em linhas com caractere fora do BMP antes do nome, a posição pode sair deslocada.
- Escopo de parâmetros e de `let` é por ordem no texto, não por bloco. Um `let` dentro de um `if` aparece depois do bloco, assim como o autocomplete já fazia.

## `spg fmt` (2026-10-11) — CONCLUÍDA (sem bump de versão)

### Decisão (a): comentários preservados
O formatador NÃO passa pelo AST nem pelo lexer comum (que descarta comentários). Ele tem um scanner de caracteres próprio (`crates/spg-fmt/src/lib.rs`) que gera átomos e comentários, e reemite o texto com a mesma sequência de tokens.

### O que ele muda (e o que NUNCA muda)
- Muda: espaços dentro da linha, indentação (4 espaços por nível de `{`, mais 1 nível por `(`/`[` em continuação), linhas em branco (runs viram 1; sem linha em branco no início/fim), espaços finais, normalização do `#>` (`#>   strict` -> `#> strict`), e o espaço antes de comentário de linha (dois espaços).
- Nunca muda: quebras de linha (o formatador nunca junta nem separa linhas; por isso as linhas de erro e de trace continuam iguais), tokens, comentários, conteúdo de strings (incluindo `${...}`, `'cruas'` e strings multi-linha, que não são reindentadas).
- Regras de espaço: operadores binários com espaço; `-`/`!` unário colado; `f(x)` sem espaço, `if (x)`/`while (x)` com espaço; `function(` (tipo e lambda) sem espaço; `array<int>`/`map<K, V>` colados; `a.b` e `1..4` colados; `x: int` e `{ x: 1 }`; `[:]`, `[]` e `{}` vazios.

### Garantia verificada (`spg_fmt::check`)
Todo arquivo escrito passa por quatro provas antes de tocar no disco:
1. os tokens do lexer são idênticos (`TokenKind`, sem comentários);
2. a lista de comentários é idêntica (texto, sem espaço final);
3. as diretivas (`friendly_terminal`, `strict`, `timeout_ms`) são idênticas;
4. formatar o resultado de novo não muda nada (ponto fixo).
Se qualquer prova falha, o arquivo fica como está e o erro é impresso. Um arquivo que não lexa é recusado.

Essa verificação já pegou um bug real durante o desenvolvimento: `&&` e `||` não estavam na lista de operadores de dois caracteres, e o formatador os separava. Corrigido, com teste (`logical_operators_stay_whole`).

### CLI
- `spg fmt <arquivo|pasta>...`: reescreve no lugar (pastas são percorridas recursivamente, em ordem).
- `spg fmt --check ...`: não escreve; lista os que mudariam; saída 1 se algum mudaria.
- `spg fmt --stdout arquivo`: imprime o resultado.
- Saídas: 0 ok; 1 erro (inclui arquivo que não lexa) ou `--check` com mudanças; 2 uso incorreto.

### Testes
- `spg-fmt`: 13 unitários (espaços, indentação, linhas em branco, comentários, strings com interpolação, unário e genéricos, lambda e tipo função, diretivas, string multi-linha, check idempotente, recusa de arquivo que não lexa, diretiva preservada, operadores lógicos).
- Suíte multi: 352 -> 353 (`fmt/x.spg`: escreve, mantém o comentário e passa no `--check`).
- Prova de regressão: formatei os 326 `.spg` dos exemplos numa cópia do repositório e rodei a suíte inteira. Resultado: 351 ok; a única falha é `err/raw_glued.spg`, cujo erro de propósito tem a coluna deslocada pela formatação (o token é o mesmo; o teste fixa a coluna do original). Os traces e as linhas de erro dos demais testes ficam idênticos. O repositório em si NÃO foi reformatado.
- Workspace: 102 unitários verdes (eram 89; +13 do spg-fmt).

### Limitações conhecidas
- Espaçamento dentro de `${ ... }` não é normalizado (a string é tratada como um bloco; sem reformatar a expressão interna).
- Comentários de bloco não existem na linguagem, então não há o que tratar.
- Alinhamento visual de comentários de linha em colunas (estilo tabela) não é preservado: cada comentário fica com dois espaços antes.
- O LSP ainda não oferece `textDocument/formatting`; a API está em `spg_fmt::check`, pronta para isso.

## Tipo opcional `T?`, `none` e `??` (2026-10-11) — CONCLUÍDA (sem bump de versão)

### Sintaxe e regras
- Tipo: `int?`, `string?`, `Ponto?`, `array<int>?`, `array<int?>`, `map<string, int?>`, campo `prox: No?`, parâmetro `x: int?`, retorno `-> int?`. `T?` aceita um `T`, um `T?` ou `none`. **Nunca o contrário**: `T?` não vai para `T` (erro de tipo, com dica de como sair).
- Literal `none`. Sozinho não tem tipo: `let x = none` é erro (`NoneNeedsType`); escreva `let x: int? = none`.
- `a ?? b`: `a` precisa ser opcional; devolve `T` se `b` é `T`, ou `T?` se `b` também é opcional/`none`. O lado direito é **preguiçoso** (só roda quando `a` é `none`; testado com contador). Associativo à direita, o operador binário mais fraco: `a ?? b ?? c`, e `x ?? y || z` lê como `x ?? (y || z)`.
- `==` / `!=`: `x == none`, `x == 3` (com `x: int?`) e `x == y` (ambos `int?`) valem. `int?` contra `string?` é erro. `<`, `+`, etc. em opcional são erro.
- `print`, `"${x}"` e `@into.string` mostram `none`. JSON: `none` vira `null` (a leitura de `null` não mudou).
- **`none` agora é palavra reservada** (mudança que quebra programas que usavam `none` como nome). Dois exemplos antigos (`ok/array_eq_empty.spg`, `ok/map_basic.spg`) foram renomeados para `vazio`.
- Não existe `T??`, nem `void?`, nem parênteses em tipo: `function(int) -> int?` é função que RETORNA `int?`.

### Estreitamento de tipo (o que torna o opcional usável)
Depois de checar `none`, o tipo da variável passa a ser `T`:
- `if (x != none) { ... }`: no bloco, `x` é `T`. `else` de `x == none` também.
- `while (x != none) { ... }`: no corpo.
- `&&` e `||`: `x != none && x > 3` (o lado direito vê `x` como `T`); `x == none || x > 3`. Também `!` e combinações.
- **Guarda**: se um ramo sempre sai (`return`, `throw`, `break`, `continue`), o OUTRO lado vale no resto do bloco: `if (x == none) { return 0 }` e daí em diante `x` é `T`. `if (a == none || b == none) { return }` estreita os dois.
- Atribuir a uma variável estreitada **desfaz** o estreitamento (conservador): `cur = cur.prox` volta `cur` a `Node?`. O novo valor é checado contra o tipo ORIGINAL (por isso `x = none` é aceito).
- Lambda captura a variável já estreitada como `T`.
- Só vale para variável simples: **não há estreitamento de caminho** (`p.prox != none` não faz `p.prox.valor` valer). Copie para uma local: `let n = p.prox`.

### Implementação
- **Lexer/AST/parser**: tokens `?`, `??`, `none` (`NoneLit`); `TypeExpr::Optional`, `Expr::NoneLit`, `BinaryOp::Coalesce`. `parse_type_expr` virou base + `?` (`void?` recusado no parse); `parse_coalesce` acima de `parse_or`. `rename.rs`/`annotate.rs` ganharam os braços novos.
- **Typecheck**: `Ty::Optional(Box<Ty>)` e `Ty::NoneLit`; `Ty::accepts(found)` é a regra única de compatibilidade (usada em `let`/`const`, atribuição, `return`, argumentos via `ty_matches`, campos, índices, struct literal). `VarInfo` ganhou `origin` e `narrowed_from`. `narrowing(cond)` (função livre) calcula (nomes não-`none` se verdadeiro, se falso); `narrow`/`unnarrow` mexem no escopo (um estreitamento é um *shadow* no escopo atual, ou substitui a ligação real se ela vive nesse escopo). `resolve_name`/`assigns_capture` usam `origin` para que estreitar uma global não a transforme em captura por valor. `coalesce`, `unify_optional` (`[1, none]` é `array<int?>`), `block_exits`/`stmt_exits`.
- **Erros**: `NoneNeedsType`, `CoalesceNeedsOptional`; toda mensagem que envolve um `T?` onde se esperava valor termina com a dica "check it with `!= none` first, or give it a default with `??`" (`TypeError::fmt` = `fmt_base` + dica).
- **Compilador/VM**: `Value::Nil`; `TypedExpr::NoneLit` vira `Const(Nil)`; `Coalesce` vira `left; JumpIfNotNone(fim); right` (opcode novo, incluído em `jump_target`, `jump_target_mut` e `patch_jump`, então o peephole remapeia sozinho). Igualdade com opcional em qualquer lado usa `EqValue`/`NotEqValue` (os opcodes de int/float/string rejeitariam `none`).
- **Runtime**: `none` onde um valor era necessário (só acontece se uma global estreitada virou `none` numa função chamada no meio) dá `unexpected none: a int was needed but the value is none (...)`, **capturável por `try`/`catch`** (antes seria "VM bug"). Isso é uma fragilidade conhecida: o estreitamento de GLOBAL não é invalidado por chamadas de função. Escolha consciente: invalidar quebraria laços de script no topo do arquivo (`while (cur != none) { print(cur.valor); cur = cur.prox }`); a falha é um erro limpo, nunca memória corrompida.
- **Ferramentas**: `spg-fmt` entende `?` colado ao tipo e `??` como operador (2 testes novos); LSP lê `int?` em assinaturas/hover (`read_type`) e oferece `none` no autocomplete (1 teste novo). A extensão do VSCode NÃO foi tocada (gramática não conhece `none`/`?`/`??`; pedido para deixar quieta).

### Testes
- Suíte multi 353 -> 365: `ok/opcional.spg` (busca devolvendo `int?`, guarda, lista ligada com `while (atual != none)`, `||` de guarda, lambda sobre variável estreitada, `??` encadeado e preguiçoso, `array<int?>`, interpolação de `none`), `ok/opt_global_catch.spg` (o caso da global anulada, pego por `catch`), e 8 erros: aritmética em `int?`, `none` sem tipo, `int?` para `int`, `??` sem opcional, `if` sem saída não estreita depois, estreitamento some após `x = none`, campo de opcional, `void?`.
- Workspace: 105 unitários (eram 102; +3 de fmt/LSP). Desempenho: fib 371/373, 393/413, 366/397, 448/408 ms (base/novo, alternado): empate dentro do ruído.

### Limitações / ideias
- Nenhuma função da biblioteca devolve opcional ainda: candidatos naturais são `@map.find`, `@array.find`, `@array.first`. Hoje `none` nasce de literais, de campos e de funções do usuário.
- `let xs: array<int?> = [none, none]` falha (o literal vira `array<none>`); com pelo menos um valor (`[1, none]`) funciona.
- Sem estreitamento de campo/índice (`a[i]`, `p.f`).

## Estado da lista do 1.20.0 (2026-10-11)
Feitos: stack trace, interpolação `"${}"`, `spg build`, tipo função/lambdas/callbacks, LSP completo (assinatura, hover, definição, parâmetros, `.string()`), `spg fmt` (com comentários), `T?`/`none`/`??`.
Pulado por escolha do usuário: página de documentação e download.
Resta para datar **1.20.0**: bump de versão (`Cargo.toml`/instalador) e release pelo `update-spg.bat`. Opcional e pequeno: `textDocument/formatting` no LSP (`spg_fmt::check` já pronto).

## Release 1.20.0 (2026-10-11) — primeiro grande update

### O que entra (tudo descrito nas seções anteriores deste arquivo, de 2026-10-10 e 2026-10-11)
1. **Stack trace** nos erros de runtime (`at f (arq.spg:linha)`), com frames repetidos agrupados e limite de profundidade de 100 mil chamadas (erro capturável em vez de abortar o processo).
2. **Interpolação** `"oi ${nome}"` (açúcar do lexer; `'cruas'` nunca interpolam; `\${` é literal).
3. **`spg build`**: executável único (o próprio `spg` + fontes embutidos).
4. **Tipo função, lambdas e callbacks** (captura por valor).
5. **LSP completo**: autocomplete (parâmetros, `.string()`), assinatura (função, método, lambda), hover, go-to-definition (inclusive arquivo importado) e **formatação** (esta sessão).
6. **`spg fmt`** (comentários preservados, quatro provas de segurança antes de gravar).
7. **`T?`, `none`, `??`** com estreitamento de tipo.
Pulado por escolha: página de documentação/download.

### O que mudou nesta sessão
- **LSP `textDocument/formatting`** (`crates/spg-lsp/src/server.rs`, dependência nova `spg-fmt`): devolve UM edit que troca o documento inteiro pelo texto de `spg_fmt::check`, ou nenhum edit quando já está formatado ou o `fmt` recusa (arquivo que não lexa, ou resultado reprovado nas provas). O fim do intervalo é a posição final do texto, em unidades UTF-16 (testado com acento e sem quebra de linha final). Anunciado como `documentFormattingProvider`.
- **Extensão VSCode** (`editors/vscode`): registra `registerDocumentFormattingEditProvider` (mesmo padrão do provedor de completion: `flush(doc)` e depois `client.request('textDocument/formatting', ...)`), e `package.json` ganhou `configurationDefaults` para `[spg]`: `editor.defaultFormatter = spaghetti.spaghetti` e **`editor.formatOnSave = true`**. Para desligar: `"[spg]": { "editor.formatOnSave": false }` (documentado no README da extensão). Descrição da extensão e do manifesto agora citam "formatting". Nada da gramática foi tocado.
- **Versão 1.20.0** em: `Cargo.toml` (workspace; `Cargo.lock` atualizado pelo build), `editors/vscode/package.json`, `README.md` (2 menções) e nome do vsix. `spg --version` -> `spg 1.20.0`.
- **`dist/spaghetti-1.20.0.vsix`** regenerado (zip com o manifesto em 1.20.0 e os arquivos atuais de `editors/vscode`; conferido byte a byte contra o fonte); o `spaghetti-0.19.0.vsix` saiu do repositório.

### Verificação
- Suíte multi: 365 ok. `cargo test --workspace`: 105 unitários ok. Servidor testado pelo protocolo em 4 cenários de formatação (bagunçado, já formatado, não lexa, acentos sem `\n` final).
- `node --check editors/vscode/extension.js` ok.

### Pendências conhecidas
- **`tools/ext-tests` não roda neste pacote**: o módulo falso `vscode` (`tools/ext-tests/node_modules/vscode`) não veio no zip (falha antes e depois desta sessão com `Cannot find module 'vscode'`). Se existir na máquina do usuário, o falso precisa ganhar `languages.registerDocumentFormattingEditProvider`, `TextEdit` e `Range` com 4 números; senão o `activate()` falha. Não foi escrito teste novo da extensão por isso.
- O provedor da extensão em si (`provideDocumentFormattingEdits`) NÃO foi exercitado dentro do VSCode real: o servidor foi testado pelo protocolo e o JS passou na checagem de sintaxe, mas o trecho que converte o edit em `vscode.TextEdit` é código novo sem execução.
- A gramática do VSCode ainda não conhece `none`, `?`, `??`, `${...}` nem `function(` como tipo.
- Itens de lista ainda abertos para depois do 1.20.0: página de docs/download; `@map.find`/`@array.find` devolvendo opcional; estreitamento de campo (`p.prox`); espaçamento dentro de `${...}` no `fmt`.

## Gramática do VSCode e pacote da extensão 1.20.0 (2026-10-11)

### Gramática (`editors/vscode/syntaxes/spg.tmLanguage.json`)
- **Interpolação**: `"oi ${expr}"` abre `meta.interpolation.spg` (com `punctuation.section.interpolation.begin/end`); o conteúdo é `meta.embedded.line.spg` e volta a ser código de verdade, então operadores, números, nomes e até outra string aninhada são pintados. Chaves dentro da expressão são contadas (`${ P { x: 1 }.x }`) e uma string aninhada com `}` (`${@str.len("}")}`) não fecha a interpolação. `\${` é escape; `$` sozinho é texto; `'cru'` nunca interpola; comentário não interpola.
- **Tipos opcionais e none**: `none` = `constant.language.none.spg` (só palavra inteira; `nonexistent` não casa); `?` = `keyword.operator.optional.spg`; `??` = `keyword.operator.coalesce.spg` (com prioridade sobre `?`).
- **Funções como valor**: `function (` sem nome = `storage.type.function.spg` (lambda ou tipo); `function nome(` continua sendo declaração; `->` já era `keyword.operator.arrow.spg`.
- **Configuração de linguagem**: `${` com `}` como par de auto-fechamento (fora de comentário).

### Testes da gramática (`tools/grammar-tests/`)
- `test.js` carrega a gramática pelo motor REAL que o VSCode usa (`vscode-textmate` + `vscode-oniguruma`, WebAssembly) e confere **42 casos de escopo** (interpolação, aninhamento, escapes, raw, comentário, opcionais, `??`, lambdas, declarações, chamadas, builtins, namespaces, floats vs. intervalo `1..4`, diretivas, `me`, `step`, booleanos, tipos e propriedades).
- Segunda checagem: tokeniza os **92 exemplos** do repositório linha por linha e confirma que nenhuma regra fica aberta no fim do arquivo (vazamento de cor pro resto do arquivo).
- `run.sh` (roda de qualquer pasta): instala as duas dependências na primeira vez e executa. Resultado: 42 ok, 0 falhas; 92 de 92 sem vazamento. Os casos falharam ANTES da mudança (16 de 42) e passaram depois: o teste pega regressão.
- `node_modules` não vai no zip (o `run.sh` recria).

### Snippets (`editors/vscode/snippets/spg.json`, 27 no total; 7 novos)
`lambda`/`fnv` (função como valor), `fntype` (tipo função), `opt`/`opt?` (tipo opcional), `letopt` (`let x: T? = none`), `ifnone`/`ifn` (bloco só com valor presente), `coalesce`/`??` (valor padrão), `interp`/`${` (string com expressão).

### Extensão (`editors/vscode/`)
- README: nova linha de "Highlighting" cobrindo interpolação, opcionais e funções de valor.
- `extension.js`, `package.json`, `language-configuration.json`, `snippets/`, `syntaxes/`: mesmos da 1.20.0 com as mudanças acima.

### Pacote `dist/spaghetti-1.20.0.vsix` (regenerado)
- Conteúdo conferido byte a byte contra `editors/vscode/` (todos os arquivos, inclusive `readme.md`, que no vsix tem nome em minúsculo).
- Manifesto: a descrição cita "highlighting for interpolation, optional types and function values".

### Verificação desta sessão
- Suíte multi 365 ok; unitários 105 ok; gramática 42/42 e 92/92; `node --check` do `extension.js`; JSON de todos os arquivos de linguagem válido.

### Pendências (continuam)
- **O VSCode real não foi aberto aqui.** A gramática é testada pelo motor de tokenização que o VSCode usa, mas a aparência final (temas, cores de `meta.embedded`) depende do tema do usuário. Vale abrir um `.spg` com interpolação e conferir.
- `tools/ext-tests/` (testes com o módulo falso `vscode`) continua sem o módulo no zip: falha com `Cannot find module 'vscode'`. Não foi corrigido nesta sessão.
- Provedor de formatação da extensão (`provideDocumentFormattingEdits`) ainda sem execução no VSCode real.
- Itens fora do release: página de docs/download; `@map.find`/`@array.find` com opcional; estreitamento de campo (`p.prox`); espaçamento dentro de `${...}` no `fmt`.
