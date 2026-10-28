#!/usr/bin/env bash
# Installs spg into ~/.local/bin and the VSCode extension, if "code" is available.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin"
mkdir -p "$bin"
cp "$here/spg" "$bin/spg"
chmod +x "$bin/spg"
# standard library (install #...) lives next to the binary
rm -rf "$bin/stdlib" && cp -r "$here/stdlib" "$bin/stdlib"
# sample programs and demo apps, next to the binary like the stdlib
rm -rf "$bin/examples" && cp -r "$here/examples" "$bin/examples"
echo "spg installed at $bin/spg"
if command -v code >/dev/null 2>&1; then
  code --install-extension "$here/spaghetti.vsix" --force && echo "VSCode extension installed."
fi
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: add $bin to your PATH (e.g. export PATH=\"\$HOME/.local/bin:\$PATH\" in ~/.bashrc)";;
esac
