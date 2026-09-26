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

  # NIP-FI (S3/S4) relay witnesses: mirrors the nextest stanza in `just
  # test-unit` — the two lists must stay in step.  Split into two steps:
  #
  #   1. Wholly-new modules (nip_fi_* and api::nip_fi) selected by substring
  #      filter.  Filters go after `--`; libtest ORs multiple filters after `--`.
  #   2. Sixty-eight exact mixed-module names (state, connection, audio::handler,
  #      router, handlers::*, config) passed with --exact so each filter is an
  #      exact match.  The exact list keeps main's unselected tests in those
  #      modules out of this step; they include the ~30s sqlx acquire timeout
  #      tests that don't belong in the infra-free unit job.
  #
  # Postgres-backed NIP-FI witnesses run in the PostgreSQL lane, not here.
  run_test_step "buzz-relay NIP-FI module tests" \
    cargo test -p buzz-relay --lib -- --nocapture \
      nip_fi_config:: nip_fi_gate:: nip_fi_session:: nip_fi_upgrade:: api::nip_fi::

  # NIP-FI mixed-module exact-name list.  The array is defined here so the
  # guard and the run step share one authoritative list.  Each entry must match
  # a test that exists in buzz-relay --lib; a renamed or deleted entry silently
  # matches nothing under libtest, so a discovery guard checks the count before
  # running.  Keep this list in step with the `just test-unit` nextest stanza
  # (Justfile:478-548).
  NIP_FI_EXACT_NAMES=(
    audio::handler::tests::admin_disconnect_nip_fi_delivers_restricted_json_then_policy_close
    audio::handler::tests::audio_expiry_sends_exact_restricted_frame_before_close
    audio::handler::tests::b1_already_expired_session_denied_at_pairing_before_admission
    audio::handler::tests::b1_mid_admission_expiry_does_not_add_peer_to_room
    audio::handler::tests::cw6_guard_release_before_commit_calls_directory_release_exactly_once
    audio::handler::tests::cw7_guard_release_before_commit_sends_clean_close_on_remote_stream
    audio::handler::tests::handle_active_audio_connection_pairing_mismatch_runs_full_audio_denial_path
    audio::handler::tests::pre_send_loop_check_cancel_emits_restricted_json_then_policy_close
    audio::handler::tests::w8_membership_check_barrier_fires_before_db_read
    audio::handler::tests::w_admin_disconnect_at_deny_check_delivers_payload_then_close
    audio::handler::tests::w_audio_deny_absent_key_passes_deny_check_reaches_membership_gate
    audio::handler::tests::w_audio_deny_active_key_refused_at_post_registration_check
    audio::handler::tests::w_audio_deny_straddle_entry_inserted_between_registration_and_check_is_caught
    config::tests::hermetic_for_test_is_deterministic
    connection::tests::b2_cancelled_connection_event_frame_not_dispatched
    connection::tests::b3_denial_precedes_restart_when_denial_already_won
    connection::tests::b3_expiry_denial_precedes_close_through_send_loop
    connection::tests::b3_root_pairing_denial_precedes_close_through_send_loop
    connection::tests::deadline_exp_is_earliest_selects_exp
    connection::tests::deadline_max_connection_lifetime_is_earliest_selects_partition
    connection::tests::deadline_no_lifetime_returns_upstream_only
    connection::tests::expiry_notice_queued_on_ctrl_before_cancel
    connection::tests::r1_denial_precedes_restart_reason_won_frame_arrives_during_recv
    handlers::auth::tests::b2_pre_cancelled_connection_never_becomes_authenticated
    handlers::auth::tests::handle_auth_pairing_mismatch_runs_full_root_denial_path
    handlers::event::tests::w_observer_permit_cancel_before_acquisition_blocks_publication
    router::tests::b4_connection_upgrade_only_no_upgrade_header_not_gated
    router::tests::b4_upgrade_only_no_connection_header_not_gated
    router::tests::deny_map_admits_key_not_in_map
    router::tests::deny_map_blocks_ws_admission_for_live_entry
    router::tests::nip_fi_enforce_audio_denies_missing_assertion_401
    router::tests::nip_fi_enforce_audio_denies_token_when_no_verifier_503
    router::tests::nip_fi_enforce_nip11_content_negotiation_serves_200_not_401
    router::tests::nip_fi_enforce_root_denies_missing_assertion_401
    router::tests::nip_fi_enforce_root_denies_token_when_no_verifier_503
    router::tests::nip_fi_enforce_ws_upgrade_with_html_accept_is_gated_401
    state::tests::auth_wins_reason_enqueues_frame_then_losing_delete_does_not
    state::tests::community_disconnect_then_nip_fi_keeps_community_deleted_reason
    state::tests::conn_manager_disconnect_nip_fi_ignores_unproven_connection
    state::tests::conn_manager_disconnect_nip_fi_is_issuer_scoped
    state::tests::conn_manager_disconnect_nip_fi_sets_authorization_denied_reason
    state::tests::delete_wins_reason_losing_auth_does_not_enqueue_frame
    state::tests::delete_wins_reason_losing_expiry_does_not_enqueue_frame
    state::tests::delete_wins_reason_losing_manager_does_not_enqueue_frame
    state::tests::delete_wins_reason_losing_pairing_does_not_enqueue_frame
    state::tests::disconnect_community_wins_reason_losing_nip_fi_does_not_enqueue_frame
    state::tests::disconnect_nip_fi_wins_reason_enqueues_frame_then_losing_delete_does_not
    state::tests::drain_all_jittered_cancels_when_restart_channel_is_full_or_closed
    state::tests::drain_all_jittered_waits_for_writer_acknowledgement_without_cancelling
    state::tests::expiry_wins_reason_enqueues_frame_then_losing_delete_does_not
    state::tests::lifecycle_cancel_does_not_enqueue_frame_but_cancels_token
    state::tests::manager_wins_reason_enqueues_frame_then_losing_delete_does_not
    state::tests::nip_fi_disconnect_audio_is_issuer_scoped
    state::tests::nip_fi_disconnect_closes_proven_audio_socket_and_sends_policy_close_reason
    state::tests::nip_fi_disconnect_closes_target_audio_only_and_preserves_collocated_peer
    state::tests::nip_fi_disconnect_does_not_close_different_pubkey_audio_socket
    state::tests::nip_fi_disconnect_does_not_close_unproven_audio_socket
    state::tests::nip_fi_disconnect_then_community_keeps_authorization_denied_reason
    state::tests::pairing_wins_reason_enqueues_frame_then_losing_delete_does_not
    state::tests::w_audio_registry_lifecycle_cancel_race_payload_precedes_audio_teardown_cancel
    state::tests::w_auth_cancel_race_payload_precedes_community_cancel
    state::tests::w_cancel_race_deny_payload_precedes_community_cancel
    state::tests::w_expiry_cancel_race_payload_precedes_community_cancel
    state::tests::w_lifecycle_cancel_race_payload_precedes_lifecycle_cancel
    state::tests::w_lifecycle_cancel_race_reverse_manager_loses_after_lifecycle_wins
    state::tests::w_manager_cancel_race_payload_precedes_community_cancel
    state::tests::w_pairing_cancel_race_payload_precedes_community_cancel
    state::tests::w_root_manager_drain_race_payload_precedes_drain_lifecycle_cancel
  )

  # Guard: every name in the array must resolve to exactly one test.  A stale
  # name silently matches nothing under libtest, so check the listed count
  # equals the array length before running.  Fails the summary if any name is
  # missing or mistyped.
  local _listed
  _listed=$(cargo test -p buzz-relay --lib -- --list --exact \
    "${NIP_FI_EXACT_NAMES[@]}" 2>/dev/null | grep -c ': test$' || true)
  if [[ "${_listed}" -ne "${#NIP_FI_EXACT_NAMES[@]}" ]]; then
    error "buzz-relay NIP-FI exact-name guard FAILED: listed ${_listed}/${#NIP_FI_EXACT_NAMES[@]} tests; a name is stale or missing"
    FAILED+=("buzz-relay NIP-FI exact-name guard")
  else
    log "buzz-relay NIP-FI exact-name guard: ${_listed}/${#NIP_FI_EXACT_NAMES[@]} names resolved"
    run_test_step "buzz-relay NIP-FI mixed-module tests" \
      cargo test -p buzz-relay --lib -- --nocapture --exact \
        "${NIP_FI_EXACT_NAMES[@]}"
  fi

  # buzz-pubsub unit tests (NIP-FI disconnect fan-out codec): infra-free,
  # mirrors the nextest path in `just test-unit` — the two lists must stay
  # in step.
  run_test_step "buzz-pubsub unit tests" \
    cargo test -p buzz-pubsub --lib -- --nocapture
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
