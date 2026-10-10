#!/bin/bash
# Extension tests: a fake `vscode` module talking to the REAL `spg lsp`.
# Run from the project root after `cargo build --release`:   bash tools/ext-tests/run.sh
cd "$(dirname "$0")/../.." || exit 1
SPG_BIN="$PWD/target/release/spg" EXT_JS="$PWD/editors/vscode/extension.js" \
  NODE_PATH="$PWD/tools/ext-tests/node_modules" node tools/ext-tests/test.js
