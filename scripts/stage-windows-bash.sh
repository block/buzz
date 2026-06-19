#!/usr/bin/env bash
set -euo pipefail

# Stage a genuine, non-WSL bash for the Windows MCP shell tool. The app is
# self-contained — we cannot assume Git for Windows is installed — so we bundle
# bash rather than probing for an install.
#
# There is no standalone "bash for Windows" upstream: working Windows bash ships
# only inside git-for-windows. We download PortableGit and keep ONLY the MSYS2
# bash runtime (`usr/` + `bin/`), dropping the `mingw64/` git-program subtree as
# one separable unit (~200MB of git.exe etc. the MCP shell never invokes). We do
# NOT trim INSIDE `usr/`: bash loads `msys-2.0.dll` and other libraries lazily,
# and load-bearing pieces (terminfo, gawk libs) live alongside the docs there, so
# a hand-trimmed copy can pass an existence check yet fail mid-command with a
# cryptic error — exactly the bug class this fixes. The retained runtime is the
# untouched, complete closure git-for-windows maintains.
#
# Self-contained (no release-binary precondition) so CI can call it directly to
# exercise the download/extract/drop path on a real Windows runner — the only
# automated gate on this logic before it ships to users.
#
# Single arg: the destination dir for the staged tree (the real MSYS2 bash lands
# at <dest>/usr/bin/bash.exe). Idempotent: a `.stage-complete` marker, written last,
# proves a whole prior stage and skips the re-download; a partial stage lacks it
# and re-extracts cleanly.
#
# PATH CONTRACT (keep byte-identical across three files):
#   - dest `git-bash` (== desktop/src-tauri/binaries/git-bash) is the
#     `bundle.resources` SOURCE in desktop/scripts/build-release-config.mjs.
#   - that resource's TARGET `git-bash` is staged next to the exe by Tauri's
#     Windows installer, and crates/buzz-dev-mcp/src/shell.rs resolves
#     `git-bash\usr\bin\bash.exe` relative to its own executable at runtime.

GIT_BASH_DIR=${1:?usage: stage-windows-bash.sh <dest-dir>}
PORTABLEGIT_VERSION="2.54.0"
PORTABLEGIT_TAG="v${PORTABLEGIT_VERSION}.windows.1"
PORTABLEGIT_EXE="PortableGit-${PORTABLEGIT_VERSION}-64-bit.7z.exe"
PORTABLEGIT_URL="https://github.com/git-for-windows/git/releases/download/${PORTABLEGIT_TAG}/${PORTABLEGIT_EXE}"

STAGE_MARKER="$GIT_BASH_DIR/.stage-complete"
if [[ -f "$STAGE_MARKER" ]]; then
    echo "PortableGit bash already staged at $GIT_BASH_DIR"
    exit 0
fi

echo "Downloading PortableGit ${PORTABLEGIT_VERSION}..."
tmp_dir=$(mktemp -d -t portablegit.XXXXXX)
trap 'rm -rf "$tmp_dir"' EXIT
tmp_sfx="$tmp_dir/portablegit.7z.exe"
extract_dir="$tmp_dir/extract"
curl -fsSL "$PORTABLEGIT_URL" -o "$tmp_sfx"
# PortableGit is a 7-Zip self-extracting archive; -o/-y are its SFX flags,
# so we don't need a separate 7z on PATH.
chmod +x "$tmp_sfx"
"$tmp_sfx" -y "-o$extract_dir"

# Keep the bash runtime whole, drop the separable git-program subtree.
rm -rf "$extract_dir/mingw64"
rm -rf "$GIT_BASH_DIR"
mkdir -p "$GIT_BASH_DIR"
cp -a "$extract_dir/." "$GIT_BASH_DIR/"

rm -rf "$tmp_dir"
trap - EXIT
[[ -f "$GIT_BASH_DIR/usr/bin/bash.exe" ]] || {
    echo "Error: PortableGit extracted but $GIT_BASH_DIR/usr/bin/bash.exe is missing" >&2
    exit 1
}
# Written last, only after cp -a and the integrity check both succeed, so it is
# positive proof the whole tree landed. An interrupted stage never writes it, so
# the idempotency skip falls through to a clean re-extract.
touch "$STAGE_MARKER"
echo "PortableGit bash staged at $GIT_BASH_DIR (mingw64/ dropped)"
