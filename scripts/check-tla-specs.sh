#!/usr/bin/env bash
set -euo pipefail

# Machine-check the TLA+ specs in docs/spec/ with TLC.
#
# Two kinds of check, with OPPOSITE pass conditions:
#
#   1. Safety models — TLC must complete with no invariant violation.
#   2. The drain-reachability probe (docs/redis-cluster-mode.md §Mechanized
#      verification) — an INVERTED verdict. `Probe_NotC == phase /= 3` is
#      checked as an "invariant" so that TLC reporting it VIOLATED proves the
#      C phase is REACHABLE from the stall state. A green run means the
#      migration deadlocks at the C-gate forever, which is the bug (mutant
#      M10: MigrateB deleted). So green = FAIL here.
#
# The inverted assertion greps for the specific invariant name rather than
# "TLC exited non-zero": a parse error or a missing-module error also exits
# non-zero, and treating that as a pass would make this check useless exactly
# when the spec is broken.

TLA_VERSION="v1.7.4"
TLA_SHA256="936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
TLA_URL="https://github.com/tlaplus/tlaplus/releases/download/${TLA_VERSION}/tla2tools.jar"

SPEC_DIR="docs/spec"
JAR="${TLA_JAR:-}"

# Modules checked on every run. Deliberately a small allow-list, not a glob:
# the older specs in docs/spec/ have state spaces that run for many minutes
# (MultiTenantRelay does not finish inside a reasonable CI budget, and
# GitOnObjectStore explores ~12.4M states), so gluing them onto every PR would
# buy flakiness and queue time rather than safety. Add a module here only with
# a measured runtime.
SAFETY_MODULES=(
  "RedisClusterFencingMigration"
)

if [[ -z "$JAR" ]]; then
  JAR="$(mktemp -d)/tla2tools.jar"
  echo "Downloading tla2tools ${TLA_VERSION}..."
  curl -sSL --retry 3 -o "$JAR" "$TLA_URL"
fi

# Pin by content, not just by tag: release assets can be re-uploaded.
# sha256sum on Linux runners, shasum on macOS/dev machines.
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$JAR" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$JAR" | cut -d' ' -f1)"
fi
if [[ "$actual" != "$TLA_SHA256" ]]; then
  echo "::error::tla2tools.jar checksum mismatch." >&2
  echo "  expected $TLA_SHA256" >&2
  echo "  actual   $actual" >&2
  exit 1
fi
echo "tla2tools ${TLA_VERSION} verified ($TLA_SHA256)"

run_tlc() {
  # $1 = module, $2 = config basename
  ( cd "$SPEC_DIR" && java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC \
      -deadlock -workers 4 -config "$2" "$1" 2>&1 )
}

failed=0

# ---- 1. Safety models: must be green. -------------------------------------
for module in "${SAFETY_MODULES[@]}"; do
  base="${module}.cfg"
  if [[ ! -f "$SPEC_DIR/$base" ]]; then
    echo "::error::missing config $SPEC_DIR/$base for module $module"
    failed=1
    continue
  fi

  echo ""
  echo "=== $module ($base): expecting NO violation ==="
  out="$(run_tlc "$module" "$base")" || true

  if grep -q "Model checking completed. No error has been found." <<<"$out"; then
    grep -E "states generated" <<<"$out" | tail -1 || true
    echo "PASS: $module"
  else
    echo "::error::$module ($base) did not complete cleanly."
    grep -E "Error|Invariant .* is violated|violated" <<<"$out" | head -5 || true
    failed=1
  fi
done

# ---- 2. Drain-reachability probe: INVERTED verdict. -----------------------
drain_cfg="$SPEC_DIR/RedisClusterFencingMigrationDrain.cfg"
if [[ -f "$drain_cfg" ]]; then
  echo ""
  echo "=== Drain-reachability probe: expecting Probe_NotC VIOLATED (= C reachable) ==="
  out="$(run_tlc "RedisClusterFencingMigration" "RedisClusterFencingMigrationDrain.cfg")" || true

  if grep -q "Invariant Probe_NotC is violated" <<<"$out"; then
    echo "PASS: phase C is reachable from the stall state (MigrateB discharges the C-gate)."
  elif grep -q "Model checking completed. No error has been found." <<<"$out"; then
    echo "::error::Drain probe is GREEN, which is a FAILURE under the inverted verdict."
    echo "::error::Phase C is UNREACHABLE from the stall state: the migration would"
    echo "::error::deadlock at the C-gate forever. This is mutant M10 — most likely"
    echo "::error::MigrateB was removed or its guard broken. See"
    echo "::error::docs/redis-cluster-mode.md §Mechanized verification."
    failed=1
  else
    # Neither verdict: parse error, missing module, bad config. Not a pass.
    echo "::error::Drain probe produced neither verdict — TLC failed to run the check."
    grep -E "Error" <<<"$out" | head -5 || true
    failed=1
  fi
fi

echo ""
if (( failed )); then
  echo "TLA+ spec checks FAILED."
  exit 1
fi
echo "All TLA+ spec checks passed."
