#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_root="$(cd "$script_dir/.." && pwd)"
lock_file="${ACP_TOOLS_LOCK_FILE:-$app_root/acp-tools.lock.json}"

# shellcheck source=lib/acp-node-wrapper.sh
source "$script_dir/lib/acp-node-wrapper.sh"

usage() {
  cat <<'USAGE'
Usage: desktop/scripts/prepare-acp-tools-resource.sh [target-triple]

Stages the locked ACP bridge tools into src-tauri/resources/acp so Tauri can
bundle them as application resources: vendored npm package trees under
resources/acp/node and executable wrappers under resources/acp/bin. The
optional target triple defaults to the Rust host target.

Note: resources/acp/bin holds a single target at a time, so staging must stay
tied to the build target.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

target="${1:-}"
ensure_args=()
if [[ -n "$target" ]]; then
  ensure_args+=(--target "$target")
else
  target="$(rustc -vV | sed -n 's|host: ||p')"
fi
if [[ -z "$target" ]]; then
  echo "Could not determine rust host target." >&2
  exit 1
fi

cache_bin_dir="$("$script_dir/ensure-acp-tools.sh" ${ensure_args[@]+"${ensure_args[@]}"} --print-bin-dir)"
cache_root="$(dirname "$(dirname "$cache_bin_dir")")"
resource_root="$app_root/src-tauri/resources/acp"
resource_bin_dir="$resource_root/bin"
resource_node_dir="$resource_root/node"
mkdir -p "$resource_bin_dir"

# Keep .gitkeep but refresh any staged tools from the lock.
find "$resource_bin_dir" -type f ! -name ".gitkeep" -delete
rm -rf "$resource_node_dir"
mkdir -p "$resource_node_dir"

# Manifest for the app's Node.js runtime doctor check, staged next to the
# bin dir so the app can resolve it as the bin dir's parent. Removed up
# front so locks with no npm-sourced tools ship no manifest and the doctor
# check stays silent.
node_runtime_manifest="$resource_root/node-runtime.json"
rm -f "$node_runtime_manifest"
node_runtime_entries=()

# Manifest of the native harness CLIs vendored inside the bundled bridges
# (e.g. `claude` inside the claude-agent-sdk native package, `codex` inside
# @openai/codex). The app resolves auth probes against these pinned binaries
# instead of user installs. Kept OUT of resources/acp/bin on purpose: that
# dir is the highest-priority segment of the agent-spawn PATH, and staging
# `claude`/`codex` there would shadow the user's CLIs inside every session.
harness_cli_manifest="$resource_root/harness-clis.json"
rm -f "$harness_cli_manifest"
harness_cli_entries=()

# Ad-hoc signing failure is a warning, not a hard stop: an unsignable Mach-O
# fragment that never executes should not sink the stage, and release builds
# re-sign everything with the real identity anyway. But it must be visible —
# a silently unsigned binary surfaces much later as Gatekeeper killing a
# subprocess mid-session, which is undiagnosable from build output.
codesign_if_darwin() {
  local file="$1"
  local output
  if [[ "$(uname -s)" == "Darwin" ]] && command -v codesign >/dev/null 2>&1; then
    if ! output="$(codesign --force --sign - "$file" 2>&1)"; then
      echo "Warning: ad-hoc codesign failed for $file — Gatekeeper may kill it at spawn time:" >&2
      echo "$output" >&2
    fi
  fi
}

while IFS=$'\t' read -r id binary package version node_engine native_package native_executable; do
  [[ -n "$id" ]] || continue
  install_dir="$cache_root/$target/$id/$version/npm"
  entrypoint="$install_dir/node_modules/$package/dist/index.js"
  if [[ ! -f "$entrypoint" ]]; then
    echo "Locked npm ACP tool missing from cache: $package@$version" >&2
    exit 1
  fi
  resource_package_dir="$resource_node_dir/$id"
  mkdir -p "$resource_package_dir"
  cp -R "$install_dir/." "$resource_package_dir/"
  resource_entrypoint="$resource_package_dir/node_modules/$package/dist/index.js"
  if [[ ! -f "$resource_entrypoint" ]]; then
    echo "Failed to stage npm ACP tool: $package@$version" >&2
    exit 1
  fi
  write_node_wrapper "$resource_bin_dir/$binary" "../node/$id/node_modules/$package/dist/index.js" "$node_engine"
  node_runtime_entries+=("$id"$'\t'"$binary"$'\t'"$node_engine"$'\t'"$(acp_required_node_major "$node_engine")")
  # Record the vendored native harness CLI (relative to the acp resource
  # root) for the auth-probe manifest. Fail loudly if the lock names one
  # that is not in the staged tree — a silent miss would quietly send auth
  # probes back to unpinned user installs.
  if [[ -n "$native_package" && -n "$native_executable" ]]; then
    cli_relpath="node/$id/node_modules/$native_package/$native_executable"
    cli_abspath="$resource_root/$cli_relpath"
    if [[ ! -f "$cli_abspath" ]]; then
      echo "Locked native harness CLI missing from staged tree: $cli_relpath" >&2
      exit 1
    fi
    chmod +x "$cli_abspath"
    harness_cli_entries+=("$id"$'\t'"$(basename "$native_executable")"$'\t'"$cli_relpath")
  fi
  # Ad-hoc sign every Mach-O in the staged package, not just the main CLIs:
  # the codex native package also vendors executables like rg and zsh, and
  # unsigned nested Mach-Os are killed by Gatekeeper. Darwin only, so Linux
  # staging skips the file(1) scan.
  if [[ "$(uname -s)" == "Darwin" ]]; then
    while IFS= read -r -d '' candidate; do
      if file -b "$candidate" | grep -q "Mach-O"; then
        codesign_if_darwin "$candidate"
      fi
    done < <(find "$resource_package_dir" -type f -print0)
  fi
done < <(node - "$lock_file" "$target" <<'NODE'
const fs = require("node:fs");
const [lockFile, target] = process.argv.slice(2);
const data = JSON.parse(fs.readFileSync(lockFile, "utf8"));
for (const entry of data.tools ?? []) {
  if (entry.target !== target || typeof entry.binary !== "string") continue;
  if (entry.source !== "npm") {
    throw new Error(`Unsupported ACP tool source: ${entry.source}`);
  }
  console.log([
    entry.id,
    entry.binary,
    entry.package,
    entry.version,
    entry.nodeEngine ?? ">=22",
    entry.nativePackage ?? "",
    entry.nativeExecutable ?? "",
  ].join("\t"));
}
NODE
)

# One manifest entry per npm-sourced bridge, each carrying its own required
# Node major, so bridges with different engine ranges surface distinct
# requirements in the doctor check.
if ((${#node_runtime_entries[@]} > 0)); then
  node -e '
const fs = require("node:fs");
const [manifestFile, ...entries] = process.argv.slice(1);
const tools = entries.map((line) => {
  const [id, binary, nodeEngine, requiredNodeMajor] = line.split("\t");
  return { id, binary, nodeEngine, requiredNodeMajor: Number(requiredNodeMajor) };
});
fs.writeFileSync(manifestFile, `${JSON.stringify({ tools }, null, 2)}\n`);
' "$node_runtime_manifest" ${node_runtime_entries[@]+"${node_runtime_entries[@]}"}
  echo "Wrote ACP Node runtime manifest: $node_runtime_manifest"
fi

# One manifest entry per vendored native harness CLI, keyed by the bare CLI
# name the app's auth probes use (`claude`, `codex`). Paths are relative to
# the acp resource root (the bin dir's parent).
if ((${#harness_cli_entries[@]} > 0)); then
  node -e '
const fs = require("node:fs");
const [manifestFile, ...entries] = process.argv.slice(1);
const clis = entries.map((line) => {
  const [id, cli, path] = line.split("\t");
  return { id, cli, path };
});
fs.writeFileSync(manifestFile, `${JSON.stringify({ clis }, null, 2)}\n`);
' "$harness_cli_manifest" ${harness_cli_entries[@]+"${harness_cli_entries[@]}"}
  echo "Wrote ACP harness CLI manifest: $harness_cli_manifest"
fi

echo "Staged ACP tools resource: $resource_bin_dir"
