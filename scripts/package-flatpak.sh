#!/usr/bin/env bash
# Wraps the Linux binary from dist/ (made by package-linux.sh) into a
# single-file Flatpak bundle: dist/BaseLens-<version>-<arch>.flatpak.
# Needs only `flatpak` and the org.freedesktop.Platform runtime:
#   flatpak install --user flathub org.freedesktop.Platform//25.08
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="$(scripts/version.sh)"
ARCH="$(uname -m)"
APP_ID=dev.baselens.BaseLens
RUNTIME=org.freedesktop.Platform
BRANCH=25.08
TGZ="dist/baselens-$VERSION-linux-$ARCH.tar.gz"
[ -f "$TGZ" ] || { echo "run scripts/package-linux.sh first ($TGZ missing)"; exit 1; }

WORK="$(mktemp -d)"
tar -C "$WORK" -xzf "$TGZ"
SRC="$WORK/baselens-$VERSION-linux-$ARCH"
BUILD="$WORK/build"
REPO="$WORK/repo"

# the Platform runtime doubles as the "SDK": nothing is compiled inside
flatpak build-init "$BUILD" "$APP_ID" "$RUNTIME" "$RUNTIME" "$BRANCH"
install -Dm755 "$SRC/baselens" "$BUILD/files/bin/baselens"
DESKTOP="$BUILD/files/share/applications/$APP_ID.desktop"
install -Dm644 "$SRC/assets/baselens.desktop" "$DESKTOP"
sed -i "s/^Icon=baselens/Icon=$APP_ID/; s/^StartupWMClass=.*/StartupWMClass=$APP_ID/" "$DESKTOP"
META="$BUILD/files/share/metainfo/$APP_ID.metainfo.xml"
install -Dm644 "packaging/linux/$APP_ID.metainfo.xml" "$META"
sed -i "s/baselens.desktop/$APP_ID.desktop/" "$META"
# Note: no scalable SVG icon — flatpak build-export validates icons through
# gdk-pixbuf, and the GitHub runner lacks the librsvg loader, so an SVG makes
# it fail with "Format not recognized". PNG sizes below cover every use.
for n in 16 32 48 64 128 256 512; do
    install -Dm644 "$SRC/assets/baselens-$n.png" "$BUILD/files/share/icons/hicolor/${n}x${n}/apps/$APP_ID.png"
done
flatpak build-finish "$BUILD" --command=baselens \
    --share=network --share=ipc \
    --socket=wayland --socket=fallback-x11 --device=dri \
    --filesystem=home --talk-name=org.freedesktop.secrets
flatpak build-export --arch="$ARCH" "$REPO" "$BUILD" stable
mkdir -p dist
flatpak build-bundle --arch="$ARCH" --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$REPO" "dist/BaseLens-$VERSION-$ARCH.flatpak" "$APP_ID" stable
ls -la dist/*.flatpak
