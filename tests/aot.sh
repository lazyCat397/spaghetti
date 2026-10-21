#!/bin/bash
# AOT check: every program in tests/ that `spg build` accepts must print the
# same output and exit code as `spg run`. Programs that do not compile are refused by
# both (checked separately). Usage: bash tests/aot.sh
cd "$(dirname "$0")"
SPG=$(realpath "${SPG:-../target/release/spg}")
diff=0; built=0
for f in $(find . -name '*.spg' | sort); do
    dir=$(dirname "$f"); base=$(basename "$f")
    (cd "$dir" && timeout 30 "$SPG" build "$base" -o /tmp/aot_exe >/dev/null 2>&1)
    [ -x /tmp/aot_exe ] || { rm -f /tmp/aot_exe; continue; }
    built=$((built+1))
    a=$(cd "$dir" && timeout 30 "$SPG" run "$base" </dev/null 2>&1; echo "exit=$?")
    b=$(cd "$dir" && timeout 30 /tmp/aot_exe </dev/null 2>&1; echo "exit=$?")
    rm -f /tmp/aot_exe
    if [ "$a" != "$b" ]; then diff=$((diff+1)); echo "FAIL $f"; echo "  run: $(echo "$a" | tail -3 | tr '\n' '|')"; echo "  exe: $(echo "$b" | tail -3 | tr '\n' '|')"; fi
done
echo "aot: built=$built diffs=$diff"
[ "$diff" = 0 ]
