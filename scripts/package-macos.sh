#!/usr/bin/env bash
# Builds a universal (Apple Silicon + Intel) BaseLens.app and a .dmg into dist/.
# Runs on macOS only (needs lipo, iconutil and hdiutil from Xcode tools):
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#   scripts/package-macos.sh
# The app is not signed: the first time, open it with right-click → Open.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(scripts/version.sh)"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
mkdir -p dist

for t in aarch64-apple-darwin x86_64-apple-darwin; do
    MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked --target "$t"
done

WORK="$(mktemp -d)"
APP="$WORK/BaseLens.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/baselens" \
    "$TARGET_DIR/aarch64-apple-darwin/release/baselens" \
    "$TARGET_DIR/x86_64-apple-darwin/release/baselens"
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# icon: .iconset from the brand PNGs → .icns
SET="$WORK/BaseLens.iconset"
mkdir -p "$SET"
for n in 16 32 128 256 512; do
    cp "assets/baselens-$n.png" "$SET/icon_${n}x${n}.png"
done
cp assets/baselens-32.png "$SET/icon_16x16@2x.png"
cp assets/baselens-64.png "$SET/icon_32x32@2x.png"
cp assets/baselens-256.png "$SET/icon_128x128@2x.png"
cp assets/baselens-512.png "$SET/icon_256x256@2x.png"
iconutil -c icns "$SET" -o "$APP/Contents/Resources/BaseLens.icns"

# ad-hoc signature so Apple Silicon runs it
codesign --force --deep --sign - "$APP"

(cd "$WORK" && zip -qry "$OLDPWD/dist/BaseLens-$VERSION-macos-universal.zip" BaseLens.app)
DMG_DIR="$WORK/dmg"
mkdir -p "$DMG_DIR"
cp -R "$APP" "$DMG_DIR/"
ln -s /Applications "$DMG_DIR/Applications"
hdiutil create -volname "BaseLens $VERSION" -srcfolder "$DMG_DIR" -ov -format UDZO "dist/BaseLens-$VERSION-macos-universal.dmg"
ls -la dist/*macos*
