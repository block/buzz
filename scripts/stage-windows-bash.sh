#!/usr/bin/env bash
set -euo pipefail

# Stage a genuine, non-WSL bash for the Windows MCP shell tool. The app is
# self-contained — we cannot assume Git for Windows is installed — so we bundle
# the full toolchain rather than probing for an install.
#
# There is no standalone "bash for Windows" upstream: working Windows bash ships
# only inside git-for-windows. We download PortableGit and keep the WHOLE tree —
# the MSYS2 bash runtime (`usr/` + `bin/`) AND the `mingw64/` subtree that carries
# `git.exe` plus `curl`/full `sed`/`awk`/`grep`/`find` — so a bare host gets a
# real dev env, not just bash + coreutils. `jq` is NOT in PortableGit, so we
# additionally vendor a pinned standalone `jq.exe` into `mingw64/bin` (see below).
# We do NOT trim INSIDE the tree: bash and git load `msys-2.0.dll` and other
# libraries lazily, and load-bearing pieces (terminfo, gawk libs in `usr/`; git
# templates, certs, DLLs in `mingw64/`) live alongside the docs, so a hand-trimmed
# copy can pass an existence check yet fail mid-command with a cryptic error —
# exactly the bug class this avoids. The retained runtime is the untouched,
# complete closure git-for-windows maintains.
#
# Self-contained (no release-binary precondition) so CI can call it directly to
# exercise the download/extract path on a real Windows runner — the only
# automated gate on this logic before it ships to users.
#
# Single arg: the destination dir for the staged tree (the launcher bash lands at
# <dest>/bin/bash.exe; git lands at <dest>/mingw64/bin/git.exe). Idempotent: a
# versioned `.stage-complete-v2` marker, written last, proves a whole prior stage
# and skips the re-download; a partial stage (or a stale v1 marker from the old
# mingw64-dropped layout) lacks it and re-extracts cleanly.
#
# PATH CONTRACT (keep byte-identical across three files):
#   - dest `git-bash` (== desktop/src-tauri/binaries/git-bash) is the
#     `bundle.resources` SOURCE in desktop/scripts/build-release-config.mjs.
#   - that resource's TARGET `git-bash` is staged next to the exe by Tauri's
#     Windows installer, and crates/buzz-dev-mcp/src/shell.rs resolves
#     `git-bash\bin\bash.exe` relative to its own executable at runtime.

GIT_BASH_DIR=${1:?usage: stage-windows-bash.sh <dest-dir>}
PORTABLEGIT_VERSION="2.54.0"
PORTABLEGIT_TAG="v${PORTABLEGIT_VERSION}.windows.1"
PORTABLEGIT_EXE="PortableGit-${PORTABLEGIT_VERSION}-64-bit.7z.exe"
PORTABLEGIT_URL="https://github.com/git-for-windows/git/releases/download/${PORTABLEGIT_TAG}/${PORTABLEGIT_EXE}"

# jq is NOT shipped in PortableGit (it is an independent MSYS2 package, not a git
# component), but agents need it for JSON piping, so we vendor the standalone
# static jq.exe (single binary, no DLL closure) into the bundle's mingw64/bin so
# it resolves alongside git/curl through the launcher. Pinned by SHA-256 — never
# an unpinned fetch.
JQ_VERSION="1.8.1"
JQ_URL="https://github.com/jqlang/jq/releases/download/jq-${JQ_VERSION}/jq-windows-amd64.exe"
JQ_SHA256="23cb60a1354eed6bcc8d9b9735e8c7b388cd1fdcb75726b93bc299ef22dd9334"

STAGE_MARKER="$GIT_BASH_DIR/.stage-complete-v2"
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

# Keep the whole extracted tree — bash runtime AND the mingw64/ git+toolchain
# subtree — so the bundle is a real self-contained dev env.
rm -rf "$GIT_BASH_DIR"
mkdir -p "$GIT_BASH_DIR"
cp -a "$extract_dir/." "$GIT_BASH_DIR/"

# Vendor the pinned standalone jq.exe into mingw64/bin (alongside git/curl) so it
# resolves through the launcher. Verify the SHA-256 before it lands — a checksum
# mismatch fails the stage so a tampered/wrong binary never reaches the bundle.
echo "Downloading jq ${JQ_VERSION}..."
jq_tmp="$tmp_dir/jq.exe"
curl -fsSL "$JQ_URL" -o "$jq_tmp"
actual_sha=$(sha256sum "$jq_tmp" | cut -d' ' -f1)
[[ "$actual_sha" == "$JQ_SHA256" ]] || {
    echo "Error: jq.exe SHA-256 mismatch: got $actual_sha, expected $JQ_SHA256" >&2
    exit 1
}
cp "$jq_tmp" "$GIT_BASH_DIR/mingw64/bin/jq.exe"

rm -rf "$tmp_dir"
trap - EXIT
# Assert the load-bearing entry points landed: the launcher bash we resolve at
# runtime, git.exe in mingw64/ (the binary the whole restore exists to ship), and
# the vendored jq.exe. A stale or partial extract that lacks any must fail the
# gate, not write the marker — otherwise the idempotency skip would lock in a
# broken tree.
for required in bin/bash.exe mingw64/bin/git.exe mingw64/bin/jq.exe; do
    [[ -f "$GIT_BASH_DIR/$required" ]] || {
        echo "Error: PortableGit extracted but $GIT_BASH_DIR/$required is missing" >&2
        exit 1
    }
done
# Written last, only after cp -a and the integrity checks all succeed, so it is
# positive proof the whole tree landed. An interrupted stage never writes it, so
# the idempotency skip falls through to a clean re-extract.
touch "$STAGE_MARKER"
echo "PortableGit full toolchain staged at $GIT_BASH_DIR (bash + git + jq + coreutils)"
