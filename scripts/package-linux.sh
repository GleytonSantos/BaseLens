#!/usr/bin/env bash
# Builds the Linux binary and the .deb, .rpm, AppImage and .tar.gz into dist/.
# Meant for the Debian 12 build image (packaging/docker/Dockerfile) so the
# binary runs on older distributions too.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(scripts/version.sh)"
ARCH="$(uname -m)"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
DIST=dist
mkdir -p "$DIST"

echo "==> cargo build --release ($ARCH)"
cargo build --release --locked
BIN="$TARGET_DIR/release/baselens"

echo "==> .deb"
cargo deb --no-build --no-strip --output "$DIST/"

echo "==> .rpm"
cargo generate-rpm --target-dir "$TARGET_DIR" --output "$DIST/"

echo "==> AppImage"
APPDIR="$(mktemp -d)/BaseLens.AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/metainfo"
install -m755 "$BIN" "$APPDIR/usr/bin/baselens"
# OpenSSL travels inside: not every distribution ships libssl.so.3
for lib in libssl.so.3 libcrypto.so.3; do
    path="$(ldconfig -p | awk -v l="$lib" '$1 == l { print $NF; exit }')"
    [ -n "$path" ] && install -m644 "$path" "$APPDIR/usr/lib/"
done
install -m644 assets/baselens.desktop "$APPDIR/usr/share/applications/baselens.desktop"
for n in 16 32 48 64 128 256 512; do
    install -Dm644 "assets/baselens-$n.png" "$APPDIR/usr/share/icons/hicolor/${n}x${n}/apps/baselens.png"
done
cp assets/baselens.desktop "$APPDIR/baselens.desktop"
cp assets/baselens-256.png "$APPDIR/baselens.png"
install -m755 packaging/linux/AppRun "$APPDIR/AppRun"
ARCH="$ARCH" appimagetool --no-appstream "$APPDIR" "$DIST/BaseLens-$VERSION-$ARCH.AppImage"

echo "==> .tar.gz"
TGZ="$(mktemp -d)/baselens-$VERSION-linux-$ARCH"
mkdir -p "$TGZ"
install -m755 "$BIN" "$TGZ/baselens"
cp -r assets LICENSE README.md "$TGZ/"
tar -C "$(dirname "$TGZ")" -czf "$DIST/baselens-$VERSION-linux-$ARCH.tar.gz" "$(basename "$TGZ")"

ls -la "$DIST"
