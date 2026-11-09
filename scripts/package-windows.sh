#!/usr/bin/env bash
# Cross-compiles BaseLens.exe (x86_64, MinGW) and zips it into dist/.
# Meant for the build image (packaging/docker/Dockerfile).
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(scripts/version.sh)"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
mkdir -p dist
cargo build --release --locked --target x86_64-pc-windows-gnu
STAGE="$(mktemp -d)/BaseLens-$VERSION-windows-x64"
mkdir -p "$STAGE"
cp "$TARGET_DIR/x86_64-pc-windows-gnu/release/baselens.exe" "$STAGE/BaseLens.exe"
cp LICENSE README.md "$STAGE/"
(cd "$(dirname "$STAGE")" && zip -qr - "$(basename "$STAGE")") > "dist/BaseLens-$VERSION-windows-x64.zip"

# installer with Start menu / desktop shortcuts and an uninstaller
if command -v makensis >/dev/null; then
    makensis -V2 -DVERSION="$VERSION" -DEXE="$STAGE/BaseLens.exe" \
        -DOUT="$PWD/dist/BaseLens-$VERSION-windows-x64-setup.exe" packaging/windows/installer.nsi
else
    echo "(makensis not found: skipping the installer)"
fi
ls -la dist/*windows*
