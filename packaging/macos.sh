#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build a macOS application bundle, and optionally a disk image.
#
#   packaging/macos.sh            build dist/Spawn Universal.app
#   packaging/macos.sh --dmg      and wrap it in a .dmg
#
# GPUI needs Metal's shader compiler, which ships with Xcode rather than the
# command line tools. If xcode-select points at the latter, set DEVELOPER_DIR
# for the build rather than switching the whole machine over:
#
#   DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer packaging/macos.sh

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
name="Spawn Universal"
bundle_id="org.spawnuniversal.app"
dist="$root/dist"
app="$dist/$name.app"

version="$(awk -F'"' '/^version/ {print $2; exit}' "$root/Cargo.toml")"

echo "==> building"
cargo build --release --manifest-path "$root/Cargo.toml"

echo "==> icons"
python3 "$root/packaging/make-icon.py" >/dev/null
iconset="$(mktemp -d)/icon.iconset"
mkdir -p "$iconset"
# The names are fixed: iconutil looks for exactly these.
for pair in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
            "128 128x128" "256 128x128@2x" "256 256x256" \
            "512 256x256@2x" "512 512x512" "1024 512x512@2x"; do
    set -- $pair
    cp "$root/packaging/icons/icon-$1.png" "$iconset/icon_$2.png"
done

echo "==> bundle"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/spawn-universal" "$app/Contents/MacOS/"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/icon.icns"
rm -rf "$(dirname "$iconset")"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$name</string>
    <key>CFBundleDisplayName</key><string>$name</string>
    <key>CFBundleExecutable</key><string>spawn-universal</string>
    <key>CFBundleIdentifier</key><string>$bundle_id</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>LSMinimumSystemVersion</key><string>10.15</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Unsigned bundles are quarantined on download. An ad-hoc signature at least
# keeps the local copy launchable without a right-click-open dance.
codesign --force --deep --sign - "$app" 2>/dev/null || \
    echo "    (could not ad-hoc sign; the app still runs from this machine)"

echo "==> $app"

if [ "${1:-}" = "--dmg" ]; then
    dmg="$dist/spawn-universal-$version.dmg"
    rm -f "$dmg"
    staging="$(mktemp -d)"
    cp -R "$app" "$staging/"
    ln -s /Applications "$staging/Applications"
    hdiutil create -volname "$name" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null
    rm -rf "$staging"
    echo "==> $dmg"
fi
