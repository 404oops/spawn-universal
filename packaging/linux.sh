#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build for Linux, and either stage a tree or install it.
#
#   packaging/linux.sh             build and stage under dist/linux
#   packaging/linux.sh --install   install for the current user
#   sudo packaging/linux.sh --system   install for everyone, with the udev rule
#
# Build dependencies on Debian and Ubuntu:
#   sudo apt install libudev-dev libwayland-dev libxkbcommon-x11-dev \
#        libvulkan-dev cmake clang

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-}"

echo "==> building"
cargo build --release --manifest-path "$root/Cargo.toml"
python3 "$root/packaging/make-icon.py" >/dev/null

case "$mode" in
    --install) prefix="$HOME/.local" ;;
    --system)  prefix="/usr/local" ;;
    *)         prefix="$root/dist/linux" ;;
esac

bin="$prefix/bin"
icons="$prefix/share/icons/hicolor"
apps="$prefix/share/applications"
mkdir -p "$bin" "$apps"

install -m 755 "$root/target/release/spawn-universal" "$bin/spawn-universal"
for n in 16 32 64 128 256 512; do
    mkdir -p "$icons/${n}x${n}/apps"
    install -m 644 "$root/packaging/icons/icon-$n.png" \
        "$icons/${n}x${n}/apps/spawn-universal.png"
done

install -m 644 "$root/packaging/spawn-universal.desktop" \
    "$apps/spawn-universal.desktop"

if [ "$mode" = "--system" ]; then
    # Without this the hidraw node is root-only and the keyboard is invisible.
    install -m 644 "$root/packaging/99-spawn.rules" /etc/udev/rules.d/99-spawn.rules
    udevadm control --reload-rules && udevadm trigger
    echo "==> udev rule installed; unplug and replug the keyboard"
elif [ "$mode" = "--install" ]; then
    echo "==> installed under $prefix"
    echo "    the udev rule still needs root:"
    echo "    sudo install -m 644 packaging/99-spawn.rules /etc/udev/rules.d/"
    echo "    sudo udevadm control --reload-rules && sudo udevadm trigger"
else
    echo "==> staged under $prefix"
fi

command -v update-desktop-database >/dev/null && \
    update-desktop-database "$apps" 2>/dev/null || true
