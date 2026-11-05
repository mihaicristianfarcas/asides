#!/bin/sh
# Installs asides from this archive, for you alone: ~/.local/lib/asides, the `asides` command in
# ~/.local/bin, its launcher and icon. `install.sh --uninstall` removes them; your history and
# settings stay. Nothing here needs your password: what typing into terminals may need, asides'
# setup asks for.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
lib=$HOME/.local/lib/asides
bin=$HOME/.local/bin
desktop=$data/applications/io.github.mihaicristianfarcas.asides.desktop
icon=$data/icons/hicolor/256x256/apps/asides.png

if [ -x "$bin/asides" ]; then
    "$bin/asides" quit >/dev/null 2>&1 || true
fi
if [ "${1:-}" = "--uninstall" ]; then
    rm -rf "$lib" "$bin/asides" "$desktop" "$icon"
    echo "Removed. Your history and settings remain."
    exit 0
fi

mkdir -p "$lib" "$bin" "$(dirname "$desktop")" "$(dirname "$icon")"
for program in asides asides-app asides-overlay asides-keys; do
    # Beside it, then over it: a program that's running can't be written to.
    cp "$here/$program" "$lib/$program.new"
    mv "$lib/$program.new" "$lib/$program"
done
ln -sf "$lib/asides" "$bin/asides"
sed "s|@EXEC@|$lib/asides-app|" "$here/asides.desktop" >"$desktop"
cp "$here/asides.png" "$icon"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$(dirname "$desktop")" || true
fi

echo "Installed. Start it with: asides"
case ":$PATH:" in
*":$bin:"*) ;;
*) echo "($bin isn't on your PATH.)" ;;
esac
