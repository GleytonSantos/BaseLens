#!/usr/bin/env bash
# Fills the version and SHA-256 checksums of the AUR, Homebrew and winget
# files in packaging/ from a published GitHub release, e.g.
#   scripts/update-channels.sh 0.1.0
# It only downloads the public release files and edits packaging/; it
# publishes nothing. Afterwards:
#   AUR:      copy packaging/aur/baselens-bin/{PKGBUILD,.SRCINFO} to the
#             aur.archlinux.org/baselens-bin.git repo, commit, push
#   Homebrew: copy packaging/homebrew/{Formula,Casks} to a
#             github.com/GleytonSantos/homebrew-baselens repo, push
#   winget:   copy packaging/winget/*.yaml to
#             manifests/g/GleytonSantos/BaseLens/<version>/ in a fork of
#             microsoft/winget-pkgs and open a pull request (or use wingetcreate)
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION="${1:-$(scripts/version.sh)}"
BASE="https://github.com/GleytonSantos/BaseLens/releases/download/v$VERSION"
WORK="$(mktemp -d)"

sha() {
    local name="$1"
    if curl -fsSL -o "$WORK/$name" "$BASE/$name"; then
        sha256sum "$WORK/$name" | cut -d' ' -f1
    else
        echo ""
    fi
}

TGZ_SHA="$(sha "baselens-$VERSION-linux-x86_64.tar.gz")"
EXE_SHA="$(sha "BaseLens-$VERSION-windows-x64-setup.exe")"
DMG_SHA="$(sha "BaseLens-$VERSION-macos-universal.dmg")"
[ -n "$TGZ_SHA" ] || { echo "release v$VERSION has no Linux tarball"; exit 1; }

python3 -I - "$VERSION" "$TGZ_SHA" "$EXE_SHA" "$DMG_SHA" <<'PY'
import re, sys, pathlib
v, tgz, exe, dmg = sys.argv[1:5]
def edit(path, subs):
    p = pathlib.Path(path)
    s = p.read_text()
    for pat, rep in subs:
        s = re.sub(pat, rep, s, flags=re.M)
    p.write_text(s)
edit("packaging/aur/baselens-bin/PKGBUILD", [
    (r"^pkgver=.*$", f"pkgver={v}"),
    (r"^pkgrel=.*$", "pkgrel=1"),
    (r"^sha256sums=\(.*\)$", f"sha256sums=('{tgz}')"),
])
edit("packaging/homebrew/Formula/baselens.rb", [
    (r'^  version ".*"$', f'  version "{v}"'),
    (r'^      sha256 ".*"$', f'      sha256 "{tgz}"'),
])
cask_sha = f'"{dmg}"' if dmg else ":no_check"
edit("packaging/homebrew/Casks/baselens.rb", [
    (r'^  version ".*"$', f'  version "{v}"'),
    (r"^  sha256 .*$", f"  sha256 {cask_sha}"),
])
for f in pathlib.Path("packaging/winget").glob("*.yaml"):
    subs = [(r"^PackageVersion: .*$", f"PackageVersion: {v}"),
            (r"/v[0-9][^/]*/BaseLens-[^/]*-windows-x64-setup\.exe", f"/v{v}/BaseLens-{v}-windows-x64-setup.exe"),
            (r"/releases/tag/v.*$", f"/releases/tag/v{v}")]
    if exe:
        subs.append((r"^    InstallerSha256: .*$", f"    InstallerSha256: {exe.upper()}"))
    edit(str(f), subs)
print("version", v)
print("linux tarball", tgz)
print("windows setup", exe or "(missing)")
print("macos dmg", dmg or "(missing: cask uses :no_check)")
PY

if command -v makepkg >/dev/null; then
    (cd packaging/aur/baselens-bin && makepkg --printsrcinfo > .SRCINFO)
    echo "wrote packaging/aur/baselens-bin/.SRCINFO"
fi
