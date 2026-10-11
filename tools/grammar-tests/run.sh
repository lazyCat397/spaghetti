#!/bin/bash
# Grammar tests: the REAL TextMate engine (vscode-textmate + Oniguruma, what VSCode uses)
# tokenizes sample lines and the scopes are checked. Needs node and npm (first run installs 2 packages).
# Run from anywhere:   bash tools/grammar-tests/run.sh
cd "$(dirname "$0")" || exit 1
[ -d node_modules ] || npm install --silent --no-audit --no-fund || exit 1
node test.js
