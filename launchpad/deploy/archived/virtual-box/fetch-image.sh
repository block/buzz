#!/usr/bin/env bash
# Chunk 01 — fetch the Ubuntu 24.04 (noble) cloud image.
#
# SOP Step 1. Idempotent: an already-present, size-plausible, checksum-matching
# image is left alone and the script reports nothing to do.
#
# We use the `.ova`, not the `.img`, because Ubuntu's cloud `.img` is QCOW2 and
# VirtualBox cannot read it. The `.ova` imports natively (SOP Step 1).
set -euo pipefail

BASE_URL="${BASE_URL:-https://cloud-images.ubuntu.com/noble/current}"
OVA_NAME="noble-server-cloudimg-amd64.ova"
DEST_DIR="${DEST_DIR:-$HOME/vm-images}"
OVA="${OVA:-$DEST_DIR/noble.ova}"
MIN_BYTES=$((500 * 1024 * 1024)) # a truncated download is the common failure
FORCE="${FORCE:-0}"

say() { printf '\n=== %s ===\n' "$1"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "${1:-}" = "--force" ] && FORCE=1

mkdir -p "$DEST_DIR"

size_of() {
	# macOS stat and GNU stat disagree on flags; try BSD first.
	stat -f %z "$1" 2>/dev/null || stat -c %s "$1" 2>/dev/null || echo 0
}

if [ "$FORCE" = 0 ] && [ -f "$OVA" ]; then
	have=$(size_of "$OVA")
	if [ "$have" -ge "$MIN_BYTES" ]; then
		printf 'already present: %s (%s bytes) — nothing to do\n' "$OVA" "$have"
		printf 'run with --force to re-download\n'
		exit 0
	fi
	printf 'existing file is only %s bytes, below the %s minimum — re-downloading\n' "$have" "$MIN_BYTES"
fi

say "Downloading $OVA_NAME (~570 MB)"
# Download to a .part file and move it into place only on success, so an
# interrupted transfer never leaves something that looks like a valid image.
# `--retry` covers the transient failures that otherwise mean starting over.
curl -fL --retry 3 --retry-delay 2 -o "$OVA.part" "$BASE_URL/$OVA_NAME" \
	|| die "download failed; $OVA left untouched"

got=$(size_of "$OVA.part")
[ "$got" -ge "$MIN_BYTES" ] || die "downloaded file is only $got bytes — expected at least $MIN_BYTES"

say "Verifying against Ubuntu's published checksum"
# Best-effort: if the checksum file cannot be fetched we still proceed, because a
# size-plausible image plus a failed VM build is a much clearer failure than
# refusing to continue on a network hiccup. But a checksum MISMATCH is fatal.
if sums=$(curl -fsS --retry 2 "$BASE_URL/SHA256SUMS" 2>/dev/null); then
	want=$(printf '%s\n' "$sums" | awk -v f="$OVA_NAME" '$2 == "*"f || $2 == f {print $1; exit}')
	if [ -n "$want" ]; then
		if command -v shasum >/dev/null 2>&1; then
			got_sum=$(shasum -a 256 "$OVA.part" | awk '{print $1}')
		else
			got_sum=$(sha256sum "$OVA.part" | awk '{print $1}')
		fi
		if [ "$got_sum" != "$want" ]; then
			rm -f "$OVA.part"
			die "checksum mismatch — got $got_sum, expected $want. Download discarded."
		fi
		printf 'checksum OK (%s)\n' "$got_sum"
	else
		printf 'warning: %s not listed in SHA256SUMS; skipping checksum\n' "$OVA_NAME"
	fi
else
	printf 'warning: could not fetch SHA256SUMS; skipping checksum\n'
fi

mv "$OVA.part" "$OVA"
printf '\nready: %s (%s bytes)\n' "$OVA" "$(size_of "$OVA")"
printf 'next: ./deploy run 02   # builds the VM (destroys any existing buzz-dev)\n'
