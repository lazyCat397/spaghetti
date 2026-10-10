#!/usr/bin/env bash
# Installs spg into ~/.local/bin and the VSCode extension, if "code" is available.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin"
mkdir -p "$bin"
cp "$here/spg" "$bin/spg"
chmod +x "$bin/spg"
echo "spg installed at $bin/spg"
if command -v code >/dev/null 2>&1; then
  code --install-extension "$here/spaghetti.vsix" --force && echo "VSCode extension installed."
fi
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: add $bin to your PATH (e.g. export PATH=\"\$HOME/.local/bin:\$PATH\" in ~/.bashrc)";;
esac
