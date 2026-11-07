# Sessao JIT: estado para continuar

## Validacao do estado entregue
- Suite tests/run.sh: 596/596
- Probes (probes-jit/): 22/22 com saida igual entre VM (--no-jit) e JIT
- Lox (lox.spg com bench.lox, test1..3, parse_heavy.lox): identico a linha de base

## O que mudou nesta sessao (crates/spg-jit, crates/spg-vm, crates/spg-compiler)
- Campos array<struct> e campos struct (2a/2b-A), leitura de arr[i] e fusao arr[i].campo escalar
- Escrita em campo array e em elemento de array de struct: fica na VM (medido mais lento no JIT)
- Log de undo: snapshots com referencia fraca (WeakObj), limite de compactacao corrigido
- Elisao de undo para objetos criados na propria chamada nativa (fresh + poda amortizada)
- Builtins de matematica nativos (28): inline quando a instrucao coincide, helpers com a expressao da VM no resto
- Builtins de string nativos: charAt, substring (funcoes compartilhadas com a VM em builtins.rs),
  contains, startsWith, endsWith, toUpper, toLower, trim
- Cache de ASCII em charAt/substring (era 74% do tempo do parser do Lox)

## Medicoes (VM / JIT, mediana de 3, jit/vm < 1 = JIT mais rapido)
- math_loop 0.08x, saf_noreset 0.49x, struct_in_struct 0.61x, undo_fresh_scalar 0.86x
- Programas curtos (10-30 ms) ficam 1.3x a 1.45x: custo de compilar tudo no inicio
- Chamadas pequenas em laco (str_leak, lox, struct_array_index) 4-8% mais lentas

## PENDENTE (prioridade para o JIT)
Compilacao preguiçosa com criterio de lucro: hoje build() declara, define e finaliza todas as
funcoes elegiveis de uma vez (crates/spg-jit/src/lib.rs, fn build). A proposta:
1. compilar uma funcao so depois de N chamadas (mesmo modelo de HOT_LOOP_ENTRIES = 50)
2. ao disparar, compilar o fecho transitivo dos chamados, em um JITModule novo
3. manter os modulos vivos enquanto o codigo existir
Risco: refatorar build() (helpers e imports) para uso em runtime. Validar com a mesma bateria.

## Proximo tema reservado
Purificar builtins (ver lista de 116 em spg-typecheck/src/builtins.rs). Nativos hoje: 44.
Pendentes: arrays nao mutantes (contains, indexOf, first, last, sum), matematica restante,
e a tabela de strings de um caractere ASCII (evita alocar em charAt).

## Observacoes
- Interpretador Lox ainda nao e acelerado pelo JIT: o tempo esta na VM (quadros, despacho).
- @math.abs(i64::MIN) retorna o minimo sem erro na VM; o arith::abs que daria erro nao e usado.
- Scripts de medicao: comparar VM e JIT com mediana de execucoes intercaladas.
