#!/usr/bin/env bash
# Builds the Arch Linux package (packaging/arch/PKGBUILD) and copies it to dist/.
# Run on Arch or an Arch-based system (makepkg). Reuses ./target if
# CARGO_TARGET_DIR is set, e.g. CARGO_TARGET_DIR=$PWD/target scripts/package-arch.sh
# Extra arguments go to makepkg: with Rust from rustup (not the `rust`
# package) add --nodeps; --nocheck skips the tests.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
mkdir -p dist
cd packaging/arch
PKGDEST="$ROOT/dist" SRCDEST="$(mktemp -d)" BUILDDIR="$(mktemp -d)" makepkg -f --noconfirm "$@"
ls -la "$ROOT"/dist/*.pkg.tar.*
