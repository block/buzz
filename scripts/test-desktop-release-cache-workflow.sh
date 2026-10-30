#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
release="$root/.github/workflows/release.yml"
proof="$root/.github/workflows/desktop-release-cache-proof.yml"
canaries=(
  "$root/.github/workflows/signed-macos-canary.yml"
  "$root/.github/workflows/macos-intel-canary.yml"
  "$root/.github/workflows/windows-canary.yml"
  "$root/.github/workflows/linux-canary.yml"
)

if grep -q 'desktop-rust-release-v1\|desktop-release-cache-key' "$release"; then
  echo "Gate 1 must not alter the release cache path" >&2
  exit 1
fi
for workflow in "${canaries[@]}"; do
  grep -q 'refs/heads/main' "$workflow"
  grep -q 'desktop-native-toolchain-id.sh' "$workflow"
  grep -q 'steps.native_toolchain.outputs.id' "$workflow"
  grep -q 'actions/cache/restore@' "$workflow"
  grep -q 'actions/cache/save@' "$workflow"
  grep -q 'steps.rust_cache.outputs.cache-hit' "$workflow"
  grep -q '!desktop/src-tauri/target/\*\*/release/bundle' "$workflow"
  if grep -q 'restore-keys:.*desktop-rust\|Swatinem/rust-cache' "$workflow"; then
    echo "release Cargo cache must use split actions with no fallback: $workflow" >&2
    exit 1
  fi
done

# GitHub expressions must enter cache-key steps through env, never by direct
# interpolation into generated shell scripts. This blocks shell injection if a
# matrix or upstream output ever becomes attacker-controlled.
python3 - "$proof" "${canaries[@]}" <<'PY'
import pathlib
import re
import sys

for filename in sys.argv[1:]:
    text = pathlib.Path(filename).read_text()
    steps = re.findall(
        r"(?ms)^      - name: Compute exact release cache key\n(.*?)(?=^      - (?:name:|uses:)|\Z)",
        text,
    )
    if not steps:
        raise SystemExit(f"cache-key step missing: {filename}")
    for step in steps:
        run = re.search(r"(?ms)^        run: \|\n(.*?)(?=^        \S|\Z)", step)
        if not run:
            raise SystemExit(f"cache-key run block missing: {filename}")
        if "${{" in run.group(1):
            raise SystemExit(f"GitHub expression interpolated into cache-key shell: {filename}")
        if "NATIVE_TOOLCHAIN_ID: ${{ steps.native_toolchain.outputs.id }}" not in step:
            raise SystemExit(f"native toolchain output not passed through env: {filename}")
PY

python3 - "$release" "$root/.github/workflows/windows-canary.yml" "$proof" <<'PY'
import pathlib
import re
import shlex
import sys


def section(text: str, header: str, end: str, label: str) -> str:
    match = re.search(rf"(?ms)^{re.escape(header)}\n(.*?)(?=^{end}|\Z)", text)
    if match is None:
        raise SystemExit(f"{label} {header!r} missing")
    return match.group(1)


def run_text(step: str, label: str) -> str:
    lines = step.splitlines()
    index = next((i for i, line in enumerate(lines) if line.startswith("        run:")), None)
    if index is None:
        raise SystemExit(f"run command missing from {label}")
    value = lines[index].split("run:", 1)[1].strip()
    if value == "|":
        body = []
        for line in lines[index + 1 :]:
            if not line.strip():
                continue
            if not line.startswith("          "):
                break
            body.append(line[10:])
        value = "\n".join(body)
    return re.sub(r"\\[ \t]*\n[ \t]*", " ", value)


def shell_lines(run: str) -> list[list[str]]:
    parsed, run = [], re.sub(r"\\[ \t]*\n[ \t]*", " ", run)
    for line in run.splitlines():
        lexer = shlex.shlex(line, posix=True, punctuation_chars=";&|")
        lexer.whitespace_split = True
        lexer.commenters = "#"
        parsed.append(list(lexer))
    return parsed


def mesh_feature(run: str, prefix: tuple[str, ...] | str, label: str) -> None:
    separators, matches = {"&&", "||", ";", "|"}, []
    for tokens in shell_lines(run):
        for i, token in enumerate(tokens):
            start = i == 0 or tokens[i - 1] in separators
            shape = token == prefix if isinstance(prefix, str) else tuple(tokens[i : i + len(prefix)]) == prefix
            if not start or not shape:
                continue
            end = next((j for j in range(i + 1, len(tokens)) if tokens[j] in separators), len(tokens))
            matches.append(tokens[i:end])
    if len(matches) != 1:
        raise SystemExit(f"expected one executable {label} command, found {len(matches)}")

    values = []
    for i, token in enumerate(matches[0]):
        if token == "--features":
            if i + 1 == len(matches[0]):
                raise SystemExit(f"--features value missing from {label}")
            values.append(matches[0][i + 1])
        elif token.startswith("--features="):
            values.append(token.split("=", 1)[1])
    if values != ["mesh-llm"]:
        raise SystemExit(f"{label} must use mesh-llm, got {values}")


def rejected(run: str, prefix: tuple[str, ...] | str, label: str) -> None:
    try:
        mesh_feature(run, prefix, label)
    except SystemExit as error:
        if not str(error).startswith(f"{label} must use mesh-llm, got "):
            raise SystemExit(f"negative control failed at the wrong guard: {error}")
        return
    raise SystemExit(f"negative control was accepted: {label}")


release_path, canary_path, proof_path = map(pathlib.Path, sys.argv[1:])
job_end = r"  [\w-]+:\s*$"
step_end = r"      - "
release_job = section(release_path.read_text(), "  release-windows:", job_end, "job")
canary_job = section(canary_path.read_text(), "  build:", job_end, "job")
proof_job = section(proof_path.read_text(), "  windows:", job_end, "job")
build = "      - name: Build Windows NSIS installer (unsigned)"
cache = "      - name: Compute exact release cache key"
runs = (
    ("Windows release build", run_text(section(release_job, build, step_end, "step"), build)),
    ("Windows canary build", run_text(section(canary_job, build, step_end, "step"), build)),
    ("Windows canary cache key", run_text(section(canary_job, cache, step_end, "step"), cache)),
    ("Windows cache proof key", run_text(section(proof_job, cache, step_end, "step"), cache)),
)
build_shape, key_shape = ("pnpm", "tauri", "build"), "KEY=$(scripts/desktop-release-cache-key.py"
for (label, run), shape in zip(runs, (build_shape, build_shape, key_shape, key_shape)):
    mesh_feature(run, shape, label)

bad_release = release_job.replace(
    runs[0][1], runs[0][1].replace("--features mesh-llm", "", 1), 1
) + "\n# --features mesh-llm"
rejected(
    run_text(section(bad_release, build, step_end, "negative step"), build),
    build_shape,
    "release decoy",
)
bad_cache = canary_job.replace("--features mesh-llm", "--features default", 1)
rejected(
    run_text(section(bad_cache, cache, step_end, "negative step"), cache),
    key_shape,
    "wrong canary cache feature",
)
PY

# Producer/proof coverage must match all four release targets and features.
for target in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu x86_64-pc-windows-msvc; do
  grep -q -- "$target" "$proof" || { echo "proof missing $target" >&2; exit 1; }
done
grep -q -- '--features mesh-llm' "$proof"
# The Intel macOS proof passes its default graph through a feature matrix.
grep -q -- 'features: default' "$proof"
[[ $(grep -c 'actions/cache/save@' "$proof") -eq 0 ]]
[[ $(grep -c 'Require exact cache hit' "$proof") -eq 3 ]]
grep -q 'refs/tags/cache-proof-' "$proof"

# Linux producer must match release's default linker, not the CI-only mold path.
if grep -q 'setup-mold\|ubuntu-24.04-mold' "$root/.github/workflows/linux-canary.yml"; then
  echo "Linux cache producer diverges from the release linker" >&2
  exit 1
fi
if grep -q 'setup-mold' "$release"; then
  echo "release linker changed; re-review cache equivalence" >&2
  exit 1
fi

echo "desktop release cache workflow contract passed"
