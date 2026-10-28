#!/bin/bash
# Regenerates tests/jit-list.txt: the test files where the JIT compiles at least one
# function (`spg --time` prints "native: N", and N >= 1). Only these files need the
# --no-jit run in CI: for the others the JIT compiles nothing, so both modes run the
# same VM code path.
# Usage (from the repository root, after `cargo build --release`):
#   bash tests/jit-list.sh           regenerate the list
#   bash tests/jit-list.sh --check   fail if the committed list is out of date
cd "$(dirname "$0")"
SPG=${SPG:-../target/release/spg}
export SPG_UI_BACKEND=null
tmp=$(mktemp)
for f in ok/*.spg lang/*.spg err/*.spg diag/*.spg ../examples/*.spg; do
    case "$f" in *time_test*|*sleep*|*ps_missing*) continue;; esac
    n=$(timeout 30 "$SPG" --time "$f" </dev/null 2>&1 | grep -oE 'native: [0-9]+' | head -1 | grep -oE '[0-9]+')
    [ "${n:-0}" -ge 1 ] && echo "$f" >> "$tmp"
done
sort "$tmp" > "$tmp.sorted"
if [ "$1" = "--check" ]; then
    if diff -u jit-list.txt "$tmp.sorted"; then echo "jit-list.txt is up to date ($(wc -l < "$tmp.sorted") files)"; rc=0
    else echo "jit-list.txt is OUT OF DATE: run 'bash tests/jit-list.sh' and commit the result"; rc=1; fi
else
    mv "$tmp.sorted" jit-list.txt; echo "jit-list.txt: $(wc -l < jit-list.txt) files"; rc=0
fi
rm -f "$tmp" "$tmp.sorted"; exit $rc
