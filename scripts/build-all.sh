#!/usr/bin/env bash
# Builds every package this computer can make into dist/:
#   Linux (.deb, .rpm, AppImage, .tar.gz) and Windows (.zip) in Docker,
#   Flatpak if `flatpak` is installed, Arch if `makepkg` is installed.
# macOS packages need a Mac: scripts/package-macos.sh (or the GitHub workflow).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
IMAGE=baselens-build
mkdir -p dist "$HOME/.cargo/registry"

docker build -t "$IMAGE" packaging/docker

# the AppImage runtime, downloaded here (the container's network may be
# unavailable) and handed to package-linux.sh through target/appimage/
mkdir -p target/appimage
if [ ! -s target/appimage/runtime-x86_64 ]; then
    curl -fsSL --retry 5 -o target/appimage/runtime-x86_64 \
        https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64 \
        || echo "(couldn't download the AppImage runtime; appimagetool will try)"
fi

run() {
    docker run --rm --user "$(id -u):$(id -g)" \
        -e HOME=/tmp -e CARGO_HOME=/tmp/cargo -e CARGO_TARGET_DIR=/src/target/docker \
        -v "$HOME/.cargo/registry:/tmp/cargo/registry" \
        -v "$ROOT:/src" -w /src "$IMAGE" "$@"
}

echo "=== Linux packages (Debian 12 base) ==="
run scripts/package-linux.sh
echo "=== Windows ==="
run scripts/package-windows.sh

if command -v flatpak >/dev/null; then
    echo "=== Flatpak ==="
    scripts/package-flatpak.sh
else
    echo "(skipping Flatpak: flatpak not installed)"
fi

if command -v makepkg >/dev/null; then
    echo "=== Arch Linux ==="
    CARGO_TARGET_DIR="$ROOT/target" scripts/package-arch.sh
else
    echo "(skipping Arch: makepkg not found)"
fi

echo "=== done ==="
ls -la dist
