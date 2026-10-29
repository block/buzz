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

  # Mesh JWKS proxy-bypass regression: infra-free loopback I/O with
  # child-process environment isolation. --lib does not select its binary.
  # Keep this fallback in step with `just test-unit`.
  run_test_step "buzz-auth mesh JWKS proxy regression" \
    cargo test -p buzz-auth --test mesh_jwks_proxy -- --nocapture

  # Cross-pod disconnect payload tests (NIP-FI and community archive fence;
  # infra-free). Mirrors `just test-unit`.
  run_test_step "buzz-pubsub conn_control tests" \
    cargo test -p buzz-pubsub --lib conn_control::tests:: -- --nocapture

  run_test_step "buzz-voice tests" \
    cargo test -p buzz-voice --lib -- --nocapture

  # Inter-relay mesh crate (infra-free). Mirrors `just test-unit`.
  run_test_step "buzz-relay-mesh tests" \
    cargo test -p buzz-relay-mesh -- --nocapture

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

  # Keep MCP lifecycle coverage in step with the nextest path.
  run_test_step "buzz-dev-mcp tests" \
    cargo test -p buzz-dev-mcp -- --nocapture


  # buzz-db migrator/lint unit tests (no infra): guard the embedded-migrator
  # invariant (exactly the consolidated 0001; cutover/backfill stays an operator
  # script, not startup state) and the tenant-scoping lints. The Postgres-backed
  # buzz-db tests are #[ignore]d; nothing here (or in integration mode below,
  # which runs `cargo test -p buzz-db` without --ignored) runs them — they need a
  # separate isolated-DB gate, so --lib keeps this step infra-free.
  run_test_step "buzz-db unit tests" \
    cargo test -p buzz-db --lib -- --nocapture
  run_test_step "buzz-db source-policy tests" \
    cargo test -p buzz-db --test observability_source -- --nocapture

  run_test_step "buzz-media storage snapshot serialization test" \
    cargo test -p buzz-media --lib bucket_index::tests::bucket_snapshot_json_round_trip_preserves_community_keys -- --exact --nocapture

  run_test_step "buzz-admin storage snapshot tests" \
    cargo test -p buzz-admin storage_snapshot -- --nocapture

  run_test_step "buzz-admin community archive command tests" \
    cargo test -p buzz-admin communities -- --nocapture

  run_test_step "buzz-admin partition audit outcome contract test" \
    cargo test -p buzz-admin tests::partition_audit_outcomes_keep_their_json_names_and_exit_codes -- --exact --nocapture

  run_test_step "buzz-admin NIP-FI binding command tests" \
    cargo test -p buzz-admin nip_fi::tests -- --nocapture

  run_test_step "buzz-admin user-deletion intake command tests" \
    cargo test -p buzz-admin user_deletions::tests -- --nocapture

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

  # Mirror the relay filters from `just test-unit`: the three handler modules,
  # storage-snapshot helpers, readiness and router unit suites, and the single
  # scoped admission regression in state::tests, plus the REQ lifecycle tests.
  run_test_step "buzz-relay push runtime tests" \
    cargo test -p buzz-relay --lib push_runtime:: -- --nocapture

  run_test_step "buzz-relay operator listener tests" \
    cargo test -p buzz-relay --lib operator_listener::tests:: -- --nocapture

  run_test_step "buzz-relay channel authorization tests" \
    cargo test -p buzz-relay --lib handlers::channel_authz:: -- --nocapture

  run_test_step "buzz-relay moderation authorization tests" \
    cargo test -p buzz-relay --lib handlers::moderation_authz:: -- --nocapture

  run_test_step "buzz-relay side-effects helper tests" \
    cargo test -p buzz-relay --lib handlers::side_effects::tests:: -- --nocapture

  run_test_step "buzz-relay coalesced refresh tests" \
    cargo test -p buzz-relay --lib handlers::coalesced_refresh::tests:: -- --nocapture

  run_test_step "buzz-relay system channel scope tests" \
    cargo test -p buzz-relay --lib handlers::system_channels::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-11 document tests" \
    cargo test -p buzz-relay --lib nip11::tests:: -- --nocapture

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

  run_test_step "buzz-relay NIP-FI shadow recorder tests" \
    cargo test -p buzz-relay --lib nip_fi_shadow::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI shadow session tests" \
    cargo test -p buzz-relay --lib nip_fi_shadow_session::tests:: -- --nocapture

  run_test_step "buzz-relay startup step tests" \
    cargo test -p buzz-relay --lib startup_steps::tests:: -- --nocapture

  run_test_step "buzz-relay mesh boot tests" \
    cargo test -p buzz-relay --lib mesh_boot::tests:: -- --nocapture

  # telemetry and config tests mutate process env. libtest runs one process
  # with many threads (nextest, used by `just test-unit`, isolates each test),
  # so run them single-threaded here. `--skip` keeps the substring filter
  # `config::tests::` from also selecting nip_fi_config, which runs above.
  run_test_step "buzz-relay telemetry tests" \
    cargo test -p buzz-relay --lib telemetry::tests:: -- --test-threads=1 --nocapture

  run_test_step "buzz-relay config tests" \
    cargo test -p buzz-relay --lib config::tests:: -- --skip nip_fi_config:: --test-threads=1 --nocapture

  # Mirror the NIP-FI (S3) stanza from `just test-unit`: module filters, then
  # each exact name. Keep this list in step with that stanza's `test(=...)`s.
  run_test_step "buzz-relay NIP-FI config tests" \
    cargo test -p buzz-relay --lib nip_fi_config:: -- --nocapture

  run_test_step "buzz-relay NIP-FI resolver tests" \
    cargo test -p buzz-relay --lib nip_fi_resolver:: -- --nocapture

  # Infra-free REQ subscription-lifecycle tests, by exact name: the rest of
  # handlers::req needs a database.
  run_test_step "buzz-relay REQ subscription lifecycle tests" \
    cargo test -p buzz-relay --lib -- --exact --nocapture \
      handlers::req::tests::timed_out_historical_read_deregisters_before_closed \
      handlers::req::tests::superseded_timeout_leaves_replacement_intact \
      handlers::req::tests::search_claim_retires_live_and_yields_to_replacement \
      handlers::req::tests::concurrent_claims_and_stale_teardowns_keep_the_last_owner \
      handlers::req::tests::timeout_closed_is_emitted_before_a_replacement_can_claim \
      handlers::req::tests::revoke_then_replacement_keeps_replacement_whole \
      handlers::req::tests::claims_after_connection_cleanup_are_refused \
      handlers::req::tests::dropped_terminal_frame_cancels_connection \
      handlers::req::tests::revoke_dropped_terminal_frame_cancels_connection \
      handlers::req::tests::count_fallback_covers_filters_that_can_match_private_deletions \
      handlers::req::tests::group_state_t_and_creator_tags_reach_sql_only_for_group_state_kinds

  # Infra-free bridge search extension parser tests, by exact name: the rest of
  # api::bridge::postgres_tests needs a database.
  run_test_step "buzz-relay bridge search extension tests" \
    cargo test -p buzz-relay --lib -- --exact --nocapture \
      api::bridge::postgres_tests::bridge_search_mode_extension_accepts_prefix_snake_or_camel_case \
      api::bridge::postgres_tests::bridge_search_sort_extension_selects_newest_only_when_named

  # Infra-free guard for the destructive invite fixture's database ownership
  # check, by exact name: the rest of api::invites::postgres_tests needs a database.
  run_test_step "buzz-relay invite fixture guard test" \
    cargo test -p buzz-relay --lib -- --exact --nocapture \
      api::invites::postgres_tests::restriction_tests::postgres_tests::destructive_fixture_rejects_shared_database_urls

  run_test_step "buzz-relay readiness tests" \
    cargo test -p buzz-relay --lib readiness::tests:: -- --nocapture

  run_test_step "buzz-relay admission regression test" \
    cargo test -p buzz-relay --lib state::tests::neither_a_confirmed_inactive_community_nor_a_failed_lookup_admits_the_socket -- --nocapture

  run_test_step "buzz-relay archive revalidation fence test" \
    cargo test -p buzz-relay --lib state::tests::periodic_revalidation_disconnects_inside_the_fenced_callback -- --exact --nocapture

  run_test_step "buzz-relay bare disconnect fail-closed test" \
    cargo test -p buzz-relay --lib state::tests::bare_community_disconnect_fails_closed_when_the_fence_is_unavailable -- --exact --nocapture

  run_test_step "buzz-relay archived close-reason test" \
    cargo test -p buzz-relay --lib connection::tests::send_loop_sends_policy_close_when_community_is_archived -- --exact --nocapture

  run_test_step "buzz-relay community close first-writer tests" \
    cargo test -p buzz-relay --lib state::tests::community_disconnect_then_nip_fi_keeps_community_deleted_reason -- --exact --nocapture

  run_test_step "buzz-relay community close suppresses losing denial frame test" \
    cargo test -p buzz-relay --lib state::tests::disconnect_community_wins_reason_losing_nip_fi_does_not_enqueue_frame -- --exact --nocapture

  run_test_step "buzz-relay router tests" \
    cargo test -p buzz-relay --lib router::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI shared core tests" \
    cargo test -p buzz-relay --lib nip_fi_core::tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI HTTP ingress tests" \
    cargo test -p buzz-relay --lib nip_fi_http::tests:: -- --nocapture

  run_test_step "buzz-relay API query parsing tests" \
    cargo test -p buzz-relay --lib api::parse_query_tests:: -- --nocapture

  run_test_step "buzz-relay profile search request tests" \
    cargo test -p buzz-relay --lib api::profiles:: -- --nocapture

  run_test_step "buzz-relay Git transport Off-mode precedence tests" \
    cargo test -p buzz-relay --lib api::git::transport::off_mode_precedence_tests:: -- --nocapture

  run_test_step "buzz-relay NIP-FI upgrade tests" \
    cargo test -p buzz-relay --lib nip_fi_upgrade:: -- --nocapture

  run_test_step "buzz-relay NIP-FI admin API tests" \
    cargo test -p buzz-relay --lib api::nip_fi:: -- --nocapture

  run_test_step "buzz-relay auth metrics contract tests" \
    cargo test -p buzz-relay --lib metrics::contract_tests:: -- --nocapture

  run_test_step "buzz-relay partition alert series tests" \
    cargo test -p buzz-relay --lib metrics::partition_alert_tests:: -- --nocapture

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
    connection::terminal_close_delivery_tests::key_pairing_mismatch_closes_with_1008_on_real_socket
    connection::terminal_close_delivery_tests::session_expiry_denial_closes_with_1008_on_real_socket
    connection::terminal_close_delivery_tests::server_holds_socket_for_client_close_reply
    connection::terminal_close_delivery_tests::server_finishes_promptly_when_client_answers_close
    connection::terminal_close_delivery_tests::stalled_cleanup_does_not_hold_socket_open
    connection::terminal_close_delivery_tests::bootstrap_drain_holds_socket_for_client_close_reply
    connection::terminal_close_delivery_tests::stalled_expiry_barrier_does_not_hold_socket_open
    connection::terminal_close_delivery_tests::blocked_frame_handler_does_not_hold_socket_open
    connection::terminal_close_delivery_tests::refused_bootstrap_holds_connection_slot_until_socket_ends
    connection::terminal_close_delivery_tests::stalled_bootstrap_holds_connection_slot
    connection::terminal_close_delivery_tests::peer_wait_replies_to_client_initiated_close
    handlers::auth::tests::b2_pre_cancelled_connection_never_becomes_authenticated
    handlers::auth::tests::fi_ban_check_error_emits_terminal_authorization_unavailable
    handlers::auth::tests::fi_root_authorization_denied_rows_emit_identical_frames
    handlers::auth::tests::fi_invalid_nip42_proof_emits_terminal_evidence_rejected
    handlers::auth::tests::handle_auth_pairing_mismatch_runs_full_root_denial_path
    handlers::auth::tests::shadow_root_auth_records_pairing_denial
    handlers::auth::tests::shadow_root_auth_records_unavailable_owner_read
    handlers::ingest::postgres_tests::relay_admin_ban_maps_to_blocked_auth_failure
    handlers::ingest::postgres_tests::relay_admin_internal_maps_to_error_not_client_fault
    handlers::auth::tests::shadow_root_invalid_nip42_retires_without_record
    handlers::auth::tests::nip42_denial_class_separates_internal_failure_from_bad_evidence
    handlers::event::tests::fanout_access::owner_only_kinds_keep_only_the_owner
    handlers::event::tests::pubsub_fanout::pubsub_owner_only_kinds_reach_only_the_owner
    handlers::event::tests::pubsub_fanout::dispatch_owner_only_kinds_reach_only_the_owner
    handlers::req::tests::p1a_huddle_liveness_req_barrier_expiry_blocks_query_and_emission
    state::tests::f3_cancellation_during_check_terminates_socket_without_waiting_for_check
    state::tests::on_not_run_runs_once_on_each_deny_arm_and_never_on_admit
    state::tests::conn_manager_disconnect_nip_fi_ignores_unproven_connection
    state::tests::conn_manager_disconnect_nip_fi_is_issuer_scoped
    state::tests::conn_manager_disconnect_nip_fi_sets_authorization_denied_reason
    state::tests::nip_fi_disconnect_audio_is_issuer_scoped
    state::tests::nip_fi_owner_disconnect_closes_owned_agent_sockets_on_root_and_audio
    state::tests::nip_fi_disconnect_closes_proven_audio_socket_and_sends_policy_close_reason
    state::tests::nip_fi_disconnect_closes_target_audio_only_and_preserves_collocated_peer
    state::tests::nip_fi_disconnect_does_not_close_different_pubkey_audio_socket
    state::tests::nip_fi_disconnect_does_not_close_unproven_audio_socket
    state::tests::community_disconnect_then_nip_fi_keeps_community_deleted_reason
    state::tests::disconnect_community_wins_reason_losing_nip_fi_does_not_enqueue_frame
    state::tests::manager_disconnect_sets_reason_enqueues_frame_then_cancels
  )
  local name
  for name in "${nip_fi_exact_tests[@]}"; do
    run_test_step "buzz-relay ${name}" \
      cargo test -p buzz-relay --lib "$name" -- --exact --nocapture
  done

  run_test_step "buzz-relay binary tests" \
    cargo test -p buzz-relay --bin buzz-relay -- --nocapture

  run_test_step "buzz-relay invitation token and cutoff contracts" \
    cargo test -p buzz-relay --lib invite_token:: -- --nocapture
  run_test_step "buzz-relay invitation cutoff configuration" \
    cargo test -p buzz-relay --lib config::invite_cutoff_tests:: -- --nocapture

  run_test_step "buzz-relay mesh receive lifetime regression" \
    cargo test -p buzz-relay --lib mesh_boot::tests::demo_echo_retains_pending_receive_across_housekeeping_ticks -- --exact --nocapture

  run_test_step "buzz-relay boot lifecycle tests" \
    cargo test -p buzz-relay --test boot_lifecycle -- --nocapture
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
