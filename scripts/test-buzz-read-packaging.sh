#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
config="$repo_root/desktop/src-tauri/tauri.conf.json"
windows_config="$repo_root/desktop/src-tauri/tauri.windows.conf.json"
bundler="$repo_root/scripts/bundle-sidecars.sh"
release="$repo_root/.github/workflows/release.yml"
ci="$repo_root/.github/workflows/ci.yml"
startup="$repo_root/desktop/src-tauri/src/lib.rs"

grep -Fq '"binaries/buzz-read"' "$config"
if grep -Fq '"binaries/buzz-read"' "$windows_config"; then
  echo "buzz-read must remain Unix-only" >&2
  exit 1
fi
grep -Fq 'SIDECARS+=(buzz-backend-kubernetes buzz-read)' "$bundler"
grep -Fq 'desktop/src-tauri/Cargo.toml --target-dir target --release --bin buzz-read' "$bundler"
grep -Fq 'touch "desktop/src-tauri/binaries/buzz-read-$TARGET"' "$ci"
grep -Fq 'run: scripts/test-buzz-read-packaging.sh' "$ci"

ensure_nest_line=$(grep -n 'if let Err(error) = ensure_nest()' "$startup" | head -1 | cut -d: -f1)
operator_server_line=$(grep -n 'start_operator_read_server(app_handle.clone())' "$startup" | head -1 | cut -d: -f1)
if [[ -z "$ensure_nest_line" || -z "$operator_server_line" || "$ensure_nest_line" -ge "$operator_server_line" ]]; then
  echo "operator read server must start after ensure_nest" >&2
  exit 1
fi

for workflow in \
  "$repo_root/.github/workflows/signed-macos-canary.yml" \
  "$repo_root/.github/workflows/linux-canary.yml"; do
  grep -Fq 'cargo build --manifest-path desktop/src-tauri/Cargo.toml --target-dir target --release --bin buzz-read' "$workflow"
  grep -Fq './scripts/bundle-sidecars.sh' "$workflow"
done

intel="$repo_root/.github/workflows/macos-intel-canary.yml"
grep -Fq 'cargo build --manifest-path desktop/src-tauri/Cargo.toml --target-dir target --release --target "$TARGET" --bin buzz-read' "$intel"
grep -Fq './scripts/bundle-sidecars.sh "$TARGET"' "$intel"

if [[ $(grep -Fc 'cargo build --manifest-path desktop/src-tauri/Cargo.toml --target-dir target --release --bin buzz-read' "$release") -ne 2 ]]; then
  echo "release workflow must build buzz-read for native macOS and Linux" >&2
  exit 1
fi
if [[ $(grep -Fc 'cargo build --manifest-path desktop/src-tauri/Cargo.toml --target-dir target --release --target "$TARGET" --bin buzz-read' "$release") -ne 1 ]]; then
  echo "release workflow must build buzz-read for Intel macOS exactly once" >&2
  exit 1
fi
if [[ $(grep -Fc './scripts/bundle-sidecars.sh' "$release") -ne 4 ]]; then
  echo "unexpected release sidecar job count" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
host=$(rustc -vV | sed -n 's|host: ||p')
source_dir="$tmp/target/release"
output_dir="$tmp/binaries"
mkdir -p "$source_dir"
for binary in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz buzz-backend-kubernetes buzz-read; do
  printf '%s' "$binary" >"$source_dir/$binary"
done
BUZZ_SIDECAR_TARGET_DIR="$tmp/target" \
  BUZZ_SIDECAR_BINARIES_DIR="$output_dir" \
  "$bundler"
for binary in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz buzz-backend-kubernetes buzz-read; do
  cmp "$source_dir/$binary" "$output_dir/${binary}-${host}"
done

echo "buzz-read packaging contract passed"
