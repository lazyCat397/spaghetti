#!/bin/bash
# setup-rust.sh — installs the Rust toolchain needed to build Spaghetti
# (spg-lexer/parser/modules/typecheck/compiler/vm/lsp/cli) in a brand-new
# session, without depending on rustup (which is usually blocked by the
# sandboxed network of the Claude environment) or on any prebuilt binary
# (which would be fragile across containers with different libc/Ubuntu
# versions).
#
# Uses plain apt-get, which only needs archive.ubuntu.com and
# security.ubuntu.com — typically already allowed on the environment network.
# Takes about 15-20 seconds.
#
# Usage: bash setup-rust.sh

set -e

if command -v rustc >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    echo "Rust is already installed:"
    rustc --version
    cargo --version
    exit 0
fi

echo "Installing rustc + cargo via apt-get..."
apt-get install -y rustc cargo

echo ""
echo "Installed successfully:"
rustc --version
cargo --version
echo ""
echo "Done. To build Spaghetti (from the folder that contains Cargo.toml):"
echo "  cargo build --release"
echo "  ./target/release/spg examples/hello.spg"
