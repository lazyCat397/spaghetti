#!/usr/bin/env bash
# Instala o spg em ~/.local/bin e a extensão do VSCode, se o "code" existir.
set -e
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin"
mkdir -p "$bin"
cp "$here/spg" "$bin/spg"
chmod +x "$bin/spg"
echo "spg instalado em $bin/spg"
if command -v code >/dev/null 2>&1; then
  code --install-extension "$here/spaghetti.vsix" --force && echo "Extensão do VSCode instalada."
fi
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Atenção: adicione $bin ao seu PATH (ex.: export PATH=\"\$HOME/.local/bin:\$PATH\" no ~/.bashrc)";;
esac
