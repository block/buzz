#!/usr/bin/env bash
# =============================================================================
# run-tests.sh — Run Buzz test suite
# =============================================================================
# Usage:
#   ./scripts/run-tests.sh              # run all tests (default)
#   ./scripts/run-tests.sh unit         # unit tests only (no infra needed)
#   ./scripts/run-tests.sh integration  # integration tests only
#   ./scripts/run-tests.sh all          # explicit all
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MODE="${1:-all}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

log()    { echo -e "${BLUE}[run-tests]${NC} $*"; }
success(){ echo -e "${GREEN}[run-tests]${NC} $*"; }
warn()   { echo -e "${YELLOW}[run-tests]${NC} $*"; }
error()  { echo -e "${RED}[run-tests]${NC} $*" >&2; }
section(){ echo -e "\n${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"; echo -e "${CYAN}  $*${NC}"; echo -e "${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"; }

cd "${REPO_ROOT}"

# ---- Load .env if present ---------------------------------------------------

if [[ -f ".env" ]]; then
  log "Loading .env..."
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
else
  # Use defaults matching docker-compose.yml
  export DATABASE_URL="postgres://buzz:buzz_dev@localhost:5432/buzz" # sadscan:disable np.postgres.1
  export PGHOST=localhost
  export PGPORT=5432
  export PGUSER=buzz
  export PGPASSWORD=buzz_dev
  export PGDATABASE=buzz
  export REDIS_URL="redis://localhost:6379"
fi

# ---- Track results ----------------------------------------------------------

declare -a PASSED=()
declare -a FAILED=()

run_test_step() {
  local name="$1"
  shift
  log "Running: ${name}"
  if "$@"; then
    success "${name} passed"
    PASSED+=("${name}")
  else
    error "${name} FAILED"
    FAILED+=("${name}")
  fi
}

# ---- Check / start infra (for integration tests) ----------------------------

ensure_infra() {
  "${REPO_ROOT}/bin/just" _ensure-migrations
}

# ---- Unit tests (no infra needed) -------------------------------------------

run_unit_tests() {
  section "Unit Tests (no infra required)"

  run_test_step "buzz-core tests" \
    cargo test -p buzz-core --lib -- --nocapture

  run_test_step "buzz-audit tests" \
    cargo test -p buzz-audit --lib -- --nocapture

  run_test_step "buzz-auth unit tests" \
    cargo test -p buzz-auth --lib -- --nocapture

  run_test_step "buzz-voice tests" \
    cargo test -p buzz-voice --lib -- --nocapture

  run_test_step "buzz-cli tests" \
    cargo test -p buzz-cli -- --nocapture

  # buzz-sdk builder/validation unit tests: pure event-builder and input
  # validation, no infra. Mirrors the nextest path in `just test-unit` — the
  # two lists must stay in step. `--lib` matches the nextest invocation and
  # avoids the full-package rustdoc dependency-resolution flake.
  run_test_step "buzz-sdk unit tests" \
    cargo test -p buzz-sdk --lib -- --nocapture

  # Keep the relay-to-agent trust-boundary regressions in the fallback path
  # when cargo-nextest is unavailable.
  run_test_step "buzz-acp tests" \
    cargo test -p buzz-acp -- --nocapture


  # buzz-db migrator/lint unit tests (no infra): guard the embedded-migrator
  # invariant (exactly the consolidated 0001; cutover/backfill stays an operator
  # script, not startup state) and the tenant-scoping lints. The Postgres-backed
  # buzz-db tests are #[ignore]d; nothing here (or in integration mode below,
  # which runs `cargo test -p buzz-db` without --ignored) runs them — they need a
  # separate isolated-DB gate, so --lib keeps this step infra-free.
  run_test_step "buzz-db unit tests" \
    cargo test -p buzz-db --lib -- --nocapture

  run_test_step "buzz-media storage snapshot serialization test" \
    cargo test -p buzz-media --lib bucket_index::tests::bucket_snapshot_json_round_trip_preserves_community_keys -- --exact --nocapture

  run_test_step "buzz-admin storage snapshot tests" \
    cargo test -p buzz-admin storage_snapshot -- --nocapture

  # Multi-tenant conformance gate: independent replay checker + golden
  # fixtures (buzz-conformance). Pure in-process trace replay, no infra.
  run_test_step "buzz-conformance tests" \
    cargo test -p buzz-conformance -- --nocapture

  run_test_step "buzz-push-gateway tests" \
    cargo test -p buzz-push-gateway -- --nocapture
  run_test_step "buzz-push-gateway personal development tests" \
    cargo test -p buzz-push-gateway --features personal-dev-app-attest -- --nocapture

  # Kubernetes backend provider: pure decision layers driven by a fake
  # substrate, no cluster. Mirrors the nextest path in `just test-unit` —
  # the two lists must stay in step or the fallback silently covers less.
  run_test_step "buzz-backend-kubernetes tests" \
    cargo test -p buzz-backend-kubernetes -- --nocapture

  # Keep fallback parity with `just test-unit`: one LaunchDarkly-feature run
  # exercises both default and feature-gated buzz-feature-flags tests.
  run_test_step "buzz-feature-flags tests (launchdarkly)" \
    cargo test -p buzz-feature-flags --features launchdarkly -- --nocapture

  # buzz-agent model-capabilities corpus: the Rust half of the cross-language
  # drift guard. model_capabilities.rs embeds scripts/model-capabilities.json +
  # scripts/normative-corpus.json via include_str! and replays the full locked
  # corpus as pure in-process tests (no infra). Mirrors the nextest path in
  # `just test-unit` — the two lists must stay in step.
  run_test_step "buzz-agent unit tests" \
    cargo test -p buzz-agent --lib -- --nocapture

  # ACP author-gate and queue tests are pure unit tests. Keep this fallback in
  # step with `just test-unit`; ignored lifecycle tests run elsewhere.
  run_test_step "buzz-acp unit tests" \
    cargo test -p buzz-acp --lib -- --nocapture

  # Mirror the three infra-free relay handler modules in `just test-unit`'s
  # nextest expression. Keep the side-effects filter pinned to `::tests::` so
  # it does not select the sibling Postgres-backed test module.
  run_test_step "buzz-relay channel authorization tests" \
    cargo test -p buzz-relay --lib handlers::channel_authz:: -- --nocapture

  run_test_step "buzz-relay moderation authorization tests" \
    cargo test -p buzz-relay --lib handlers::moderation_authz:: -- --nocapture

  run_test_step "buzz-relay side-effects helper tests" \
    cargo test -p buzz-relay --lib handlers::side_effects::tests:: -- --nocapture

  run_test_step "buzz-relay storage snapshot tests" \
    cargo test -p buzz-relay --lib storage_sweep::tests:: -- --nocapture

  # Mirror the four audio/FI suites from `just test-unit`'s nextest expression.
  # These are infra-free (no DB, no Redis); the `#[ignore]`-gated DB witnesses
  # are excluded by cargo test's default filter. Keep in step with the Justfile
  # `test-unit` relay expression.
  run_test_step "buzz-relay audio join tests" \
    cargo test -p buzz-relay --lib audio::join::tests:: -- --nocapture

  run_test_step "buzz-relay audio handler tests" \
    cargo test -p buzz-relay --lib audio::handler::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI gate tests" \
    cargo test -p buzz-relay --lib nip_fi_gate::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI session tests" \
    cargo test -p buzz-relay --lib nip_fi_session::tests:: -- --nocapture

  # Mirror the NIP-FI (S3) stanza from `just test-unit`: module filters, then
  # each exact name. Keep this list in step with that stanza's `test(=...)`s.
  run_test_step "buzz-relay NIP-FI config tests" \
    cargo test -p buzz-relay --lib nip_fi_config:: -- --nocapture

  run_test_step "buzz-relay router tests" \
    cargo test -p buzz-relay --lib router::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI HTTP ingress tests" \
    cargo test -p buzz-relay --lib nip_fi_http::tests:: -- --nocapture

  run_test_step "buzz-relay API query parsing tests" \
    cargo test -p buzz-relay --lib api::parse_query_tests:: -- --nocapture

  run_test_step "buzz-relay Git transport Off-mode precedence tests" \
    cargo test -p buzz-relay --lib api::git::transport::off_mode_precedence_tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI upgrade tests" \
    cargo test -p buzz-relay --lib nip_fi_upgrade:: -- --nocapture

  run_test_step "buzz-relay auth metrics contract tests" \
    cargo test -p buzz-relay --lib metrics::contract_tests:: -- --nocapture

  local nip_fi_exact_tests=(
    audio::room::tests::roster_revisions_are_ordered_and_snapshot_is_authoritative
    connection::tests::auth_lifecycle_reconciles_every_terminal_and_never_leaks_gauge
    audio::room::tests::b1_pending_peer_removed_before_commit_emits_no_delta
    audio::room::tests::b2_commit_peer_emits_exactly_one_joined_delta_and_marks_visible
    audio::room::tests::b3_commit_peer_revision_is_monotone_between_concurrent_events
    audio::room::tests::f7a_pending_peer_excluded_from_snapshot_until_committed
    connection::tests::b2_cancelled_connection_event_frame_not_dispatched
    connection::tests::b3_expiry_denial_precedes_close_through_send_loop
    connection::tests::b3_root_pairing_denial_precedes_close_through_send_loop
    connection::tests::cancellation_during_select_with_fi_denial_routes_through_bounded_path
    connection::tests::cancelled_never_ready_sink_with_queued_fi_denial_exits_within_timeout
    connection::tests::deadline_exp_is_earliest_selects_exp
    connection::tests::deadline_max_connection_lifetime_is_earliest_selects_partition
    connection::tests::deadline_no_lifetime_returns_upstream_only
    connection::tests::expiry_notice_queued_on_ctrl_before_cancel
    connection::tests::f3_root_outer_wrapper_delivers_denial_on_bootstrap_cancellation
    connection::tests::f3_root_pre_built_expired_gate_terminates_connection
    handlers::auth::tests::b2_pre_cancelled_connection_never_becomes_authenticated
    handlers::auth::tests::fi_ban_check_error_emits_terminal_authorization_unavailable
    handlers::auth::tests::fi_invalid_nip42_proof_emits_terminal_evidence_rejected
    handlers::auth::tests::handle_auth_pairing_mismatch_runs_full_root_denial_path
    handlers::auth::tests::nip42_denial_class_separates_internal_failure_from_bad_evidence
    handlers::event::tests::p1b_agent_observer_event_barrier_expiry_blocks_fanout_and_ack
    handlers::req::tests::p1a_huddle_liveness_req_barrier_expiry_blocks_query_and_emission
    state::tests::f3_cancellation_during_check_terminates_socket_without_waiting_for_check
  )
  local name
  for name in "${nip_fi_exact_tests[@]}"; do
    run_test_step "buzz-relay ${name}" \
      cargo test -p buzz-relay --lib "$name" -- --exact --nocapture
  done

  run_test_step "buzz-relay binary tests" \
    cargo test -p buzz-relay --bin buzz-relay -- --nocapture
}

# ---- DB / integration tests (infra required) --------------------------------

run_integration_tests() {
  section "Integration Tests (requires running services)"

  ensure_infra

  run_test_step "buzz-db tests" \
    cargo test -p buzz-db -- --nocapture

  if find crates/buzz-auth/tests -maxdepth 1 -name '*.rs' -print -quit 2>/dev/null | grep -q .; then
    run_test_step "buzz-auth integration tests" \
      cargo test -p buzz-auth --test '*' -- --nocapture
  else
    run_test_step "buzz-auth (no integration tests found)" true
  fi

  run_test_step "workspace integration tests" \
    cargo test --test '*' -- --nocapture 2>/dev/null || \
    run_test_step "workspace integration tests (none found)" true
}

# ---- Main -------------------------------------------------------------------

START_TIME=$(date +%s)

case "${MODE}" in
  unit)
    run_unit_tests
    ;;
  integration)
    run_integration_tests
    ;;
  all|*)
    run_unit_tests
    run_integration_tests
    ;;
esac

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

# ---- Summary ----------------------------------------------------------------

section "Test Summary"
echo ""
echo -e "  Duration: ${ELAPSED}s"
echo ""

if [[ ${#PASSED[@]} -gt 0 ]]; then
  echo -e "  ${GREEN}Passed (${#PASSED[@]}):${NC}"
  for t in "${PASSED[@]}"; do
    echo -e "    ${GREEN}pass${NC} ${t}"
  done
fi

if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo ""
  echo -e "  ${RED}Failed (${#FAILED[@]}):${NC}"
  for t in "${FAILED[@]}"; do
    echo -e "    ${RED}fail${NC} ${t}"
  done
  echo ""
  exit 1
fi

echo ""
success "All tests passed!"
exit 0
