#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
known_good='e18b497796c12c097a38f9edb9d0641fb99eee32'
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/.github/workflows"
cp "$root"/.github/workflows/*.yml "$tmp/.github/workflows/"
for workflow in "$root"/.github/workflows/*.yaml; do
  [[ -e "$workflow" ]] || continue
  cp "$workflow" "$tmp/.github/workflows/"
done
cp "$root/renovate.json" "$tmp/renovate.json"

run_contract() {
  BUZZ_RUST_CACHE_CONTRACT_ROOT="$tmp" "$root/scripts/test-rust-cache-contract.sh"
}

expect_failure() {
  local expected=$1
  local output
  if output=$(run_contract 2>&1); then
    echo "expected rust cache contract failure containing: $expected" >&2
    exit 1
  fi
  if [[ "$output" != *"$expected"* ]]; then
    printf 'unexpected rust cache contract error:\n%s\n' "$output" >&2
    exit 1
  fi
}

write_cache_workflow() {
  cat > "$tmp/.github/workflows/new-cache-user.yaml"
}

run_contract >/dev/null

write_cache_workflow <<'YAML'
name: Bad digest
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6
YAML
expect_failure 'restored rust-cache v2.9.2'

write_cache_workflow <<'YAML'
name: Mutable tag
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - uses: Swatinem/rust-cache@v2
YAML
expect_failure 'rust-cache must stay on the v2.9.1 digest'

write_cache_workflow <<'YAML'
name: Expression ref
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - uses: Swatinem/rust-cache@${{ matrix.cache-ref }}
YAML
expect_failure 'rust-cache must stay on the v2.9.1 digest'

write_cache_workflow <<'YAML'
name: Spaced key
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - uses : Swatinem/rust-cache@v2
YAML
expect_failure 'rust-cache must stay on the v2.9.1 digest'

write_cache_workflow <<'YAML'
name: Flow mapping
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - { uses: Swatinem/rust-cache@v2 }
YAML
expect_failure 'rust-cache must stay on the v2.9.1 digest'

write_cache_workflow <<'YAML'
name: Quoted key
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - "uses": Swatinem/rust-cache@v2
YAML
expect_failure 'rust-cache must stay on the v2.9.1 digest'
rm "$tmp/.github/workflows/new-cache-user.yaml"

mkdir -p "$tmp/.github/actions/cache"
cat > "$tmp/.github/actions/cache/action.yml" <<'YAML'
name: Unsafe cache wrapper
runs:
  using: composite
  steps:
    - { uses: Swatinem/rust-cache@v2 }
YAML
write_cache_workflow <<'YAML'
name: Local cache wrapper
on: workflow_dispatch
jobs:
  cache:
    runs-on: ubuntu-latest
    steps:
      - uses: ./.github/actions/cache
YAML
expect_failure 'rust-cache must stay on the v2.9.1 digest'

cat > "$tmp/.github/actions/cache/action.yml" <<YAML
name: Safe preferred cache wrapper
runs:
  using: composite
  steps:
    - uses: Swatinem/rust-cache@$known_good
YAML
cat > "$tmp/.github/actions/cache/action.yaml" <<'YAML'
name: Unreachable stale cache wrapper
runs:
  using: composite
  steps:
    - uses: Swatinem/rust-cache@v2
YAML
run_contract >/dev/null
rm -rf "$tmp/.github/actions" "$tmp/.github/workflows/new-cache-user.yaml"

python3 - "$tmp/.github/workflows/_ci-rust.yml" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
workflow = path.read_text()
needle = "          key: sherpa-cache-v1\n          save-if:"
replacement = "          save-if:"
if workflow.count(needle) != 1:
    raise SystemExit("expected one Unit Tests cache generation key")
workflow = workflow.replace(needle, replacement)
job = "  unit-tests:\n"
if workflow.count(job) != 1:
    raise SystemExit("expected one Unit Tests job")
workflow = workflow.replace(job, job + "    env:\n      key: sherpa-cache-v1\n", 1)
path.write_text(workflow)
PY
expect_failure 'rust-cache action must keep with.key set to sherpa-cache-v1'

echo "rust cache contract regressions passed"
