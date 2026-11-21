#!/usr/bin/env node
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const BUZZ_RELAY_NEXTEST_FILTERS = [
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "--bin",
      "buzz-relay",
      "-E",
      "test(/^api::media_access::tests::/) + test(/^handlers::media_access::tests::/) + test(/^subscription::tests::/) + test(/^handlers::multinode_fanout::tests::/) + test(/^api::admin::/) + test(/^access::tests::/) + test(/^api::git::transport::access_log_tests::/) + test(=api::bridge::postgres_tests::nip98_replay_check_logs_only_its_refusal) + test(=api::bridge::postgres_tests::warn_only_artifact_read_lines_keep_the_filter_summary) + test(=connection::tests::close_totals_count_frames_the_writer_flushed) + test(=connection::tests::dispatcher_counts_each_req_and_count_frame_once) + test(=handlers::req::tests::unqueued_eose_logs_aborted_not_served) + test(=api::bridge::postgres_tests::reject_code_keeps_only_a_known_prefix) + test(=api::bridge::postgres_tests::nip98_replay_check_fails_closed_when_guard_errors) + test(=api::bridge::postgres_tests::verify_bridge_auth_rejects_nip98_event_signed_for_wrong_communitys_host) + test(=api::bridge::postgres_tests::verify_bridge_auth_can_require_payload_tag_for_json_body_endpoints) + test(=api::bridge::postgres_tests::verify_bridge_auth_accepts_nip98_event_signed_for_matching_host) + test(=api::bridge::postgres_tests::moderation_read_query_bearing_nip98_event_verifies_with_matching_query) + test(=api::bridge::postgres_tests::moderation_read_query_bearing_nip98_event_rejected_against_bare_path) + test(=api::bridge::postgres_tests::moderation_read_audit_query_bearing_nip98_event_verifies) + test(=api::bridge::postgres_tests::moderation_read_restricted_no_query_still_verifies) + test(=connection::tests::aborting_lifecycle_owner_is_drop_safe) + test(=connection::tests::shutdown_terminalizes_auth_stalled_on_database_before_late_success) + test(=connection::terminal_close_delivery_tests::upgrade_would_deny_line_without_auth_logs_absent_at_close) + test(=handlers::event::tests::observer_owner_cache_is_scoped_to_community) + test(=state::tests::shared_counter_between_direct_and_fanout) + test(=connection::tests::startup_cancellation_writes_one_close_line) + test(=connection::tests::connection_limit_refusal_writes_one_close_line) + test(=connection::tests::dropped_authenticated_handler_writes_one_close_line) + test(=connection::tests::missed_pongs_close_with_their_reason_and_count) + test(=connection::tests::client_reply_to_a_jittered_restart_closes_as_shutdown) + test(=connection::tests::client_close_racing_a_policy_disconnect_keeps_the_policy_reason) + test(=connection::tests::server_close_reason_ranks_shutdown_policy_then_cancellation) + test(=connection::tests::auth_timeout_after_a_rejected_auth_closes_as_auth_failed) + test(=connection::tests::close_totals_count_a_batch_flushed_by_the_drain_after_cancellation) + test(=connection::tests::close_totals_skip_a_frame_the_socket_rejects) + test(=connection::tests::close_totals_skip_frames_stranded_by_a_failed_flush) + test(=connection::tests::read_before_login_logs_one_denial_from_the_handler) + test(=connection::tests::read_refused_by_a_full_handler_semaphore_logs_one_denial) + test(=connection::tests::saturated_handler_write_line_records_whether_its_ok_was_queued) + test(=connection::tests::read_refused_by_unavailable_admission_logs_one_denial) + test(=handlers::auth::tests::repeat_auth_after_a_terminal_logs_one_denial) + test(=nip_fi_upgrade::tests::deny_protected_logs_one_denied_auth_line) + test(=api::bridge::moderation_refusal_tests::moderation_lookup_failure_is_unavailable) + test(/^push_runtime::/) + test(/^operator_listener::tests::/) + test(/^handlers::channel_authz::/) + test(/^handlers::moderation_authz::/) + test(/^handlers::side_effects::tests::/) + test(/^handlers::coalesced_refresh::tests::/) + test(/^handlers::system_channels::tests::/) + test(/^nip11::tests::/) + test(/^storage_sweep::tests::/) + test(/^nip_fi_core::tests::/) + test(/^nip_fi_http::tests::/) + test(/^nip_fi_policy::tests::/) + test(/^nip_fi_config::tests::/) + test(/^nip_fi_resolver::tests::/) + test(/^readiness::tests::/) + test(/^router::tests::/) + test(/^api::parse_query_tests::/) + test(/^api::db_read_error_tests::/) + test(/^api::profiles::/) + test(/^api::bridge::search_slots::tests::/) + test(/^api::git::transport::off_mode_precedence_tests::/) + test(/^api::git::store::tests::/) + test(/^api::git::store::fault_probe::/) + test(/^api::git::cas_publish::tests::/) + test(/^api::git::transport::track_c_tests::/) + test(/^audio::join::tests::/) + test(/^audio::handler::tests::/) + test(/^nip_fi_gate::tests::/) + test(/^nip_fi_session::tests::/) + test(/^nip_fi_shadow::tests::/) + test(/^nip_fi_shadow_session::tests::/) + test(/^startup_steps::tests::/) + test(/^mesh_boot::tests::/) + test(/^telemetry::tests::/) + test(/^config::tests::/) + test(=state::tests::accessible_channel_invalidation_is_scoped_to_community) + test(=state::tests::accessible_channels_join_member_and_open_halves) + test(=state::tests::channel_deleted_invalidation_is_scoped_to_community) + test(=state::tests::neither_a_confirmed_inactive_community_nor_a_failed_lookup_admits_the_socket) + test(=state::tests::periodic_revalidation_disconnects_inside_the_fenced_callback) + test(=state::tests::bare_community_disconnect_fails_closed_when_the_fence_is_unavailable) + test(=connection::tests::send_loop_sends_policy_close_when_community_is_archived) + test(=state::tests::community_disconnect_then_nip_fi_keeps_community_deleted_reason) + test(=state::tests::disconnect_community_wins_reason_losing_nip_fi_does_not_enqueue_frame) + test(=handlers::req::tests::timed_out_historical_read_deregisters_before_closed) + test(=handlers::req::tests::superseded_timeout_leaves_replacement_intact) + test(=handlers::req::tests::search_claim_retires_live_and_yields_to_replacement) + test(=handlers::req::tests::failed_search_closes_without_eose_or_trying_later_filters) + test(=handlers::req::tests::concurrent_claims_and_stale_teardowns_keep_the_last_owner) + test(=handlers::req::tests::timeout_closed_is_emitted_before_a_replacement_can_claim) + test(=handlers::req::tests::revoke_then_replacement_keeps_replacement_whole) + test(=handlers::req::tests::revoke_of_a_replaced_holder_spares_the_pending_replacement) + test(=handlers::req::tests::superseded_or_closed_requests_cannot_claim) + test(=connection::tests::close_fences_a_req_whose_handler_has_not_run) + test(=connection::tests::only_the_newest_same_id_req_answers) + test(=handlers::req::tests::claims_after_connection_cleanup_are_refused) + test(=handlers::req::tests::dropped_terminal_frame_cancels_connection) + test(=handlers::req::tests::revoke_dropped_terminal_frame_cancels_connection) + test(=handlers::req::tests::count_fallback_covers_filters_that_can_match_private_deletions) + test(=handlers::req::tests::count_fallback_fetches_one_extra_candidate) + test(=handlers::req::tests::group_state_t_and_creator_tags_reach_sql_only_for_group_state_kinds) + test(=handlers::req::tests::empty_filter_sets_match_nothing_in_sql) + test(=api::bridge::postgres_tests::bridge_search_mode_extension_accepts_prefix_snake_or_camel_case) + test(=api::bridge::postgres_tests::bridge_search_sort_extension_selects_newest_only_when_named) + test(=api::invites::postgres_tests::restriction_tests::postgres_tests::destructive_fixture_rejects_shared_database_urls) + (kind(bin) & (test(/^tests::/) + test(/^composition_tests::/) + test(/^env_filter_tests::/)) - test(/^tests::postgres_tests::/))",
    ],
  },
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "-E",
      [
        "test(/^nip_fi_upgrade::/)",
        "test(/^metrics::contract_tests::/)",
        "test(/^metrics::partition_alert_tests::/)",
        "test(=audio::room::tests::roster_revisions_are_ordered_and_snapshot_is_authoritative)",
        "test(=connection::tests::auth_lifecycle_reconciles_every_terminal_and_never_leaks_gauge)",
        "test(=audio::room::tests::b1_pending_peer_removed_before_commit_emits_no_delta)",
        "test(=audio::room::tests::b2_commit_peer_emits_exactly_one_joined_delta_and_marks_visible)",
        "test(=audio::room::tests::b3_commit_peer_revision_is_monotone_between_concurrent_events)",
        "test(=audio::room::tests::f7a_pending_peer_excluded_from_snapshot_until_committed)",
        "test(=connection::tests::b2_cancelled_connection_event_frame_not_dispatched)",
        "test(=connection::tests::b3_expiry_denial_precedes_close_through_send_loop)",
        "test(=connection::tests::b3_root_pairing_denial_precedes_close_through_send_loop)",
        "test(=connection::tests::cancellation_during_select_with_fi_denial_routes_through_bounded_path)",
        "test(=connection::tests::cancelled_never_ready_sink_with_queued_fi_denial_exits_within_timeout)",
        "test(=connection::tests::deadline_exp_is_earliest_selects_exp)",
        "test(=connection::tests::deadline_max_connection_lifetime_is_earliest_selects_partition)",
        "test(=connection::tests::deadline_no_lifetime_returns_upstream_only)",
        "test(=connection::tests::expiry_notice_queued_on_ctrl_before_cancel)",
        "test(=connection::tests::f3_root_outer_wrapper_delivers_denial_on_bootstrap_cancellation)",
        "test(=connection::tests::f3_root_pre_built_expired_gate_terminates_connection)",
        "test(=connection::terminal_close_delivery_tests::key_pairing_mismatch_closes_with_1008_on_real_socket)",
        "test(=connection::terminal_close_delivery_tests::session_expiry_denial_closes_with_1008_on_real_socket)",
        "test(=connection::terminal_close_delivery_tests::server_holds_socket_for_client_close_reply)",
        "test(=connection::terminal_close_delivery_tests::server_finishes_promptly_when_client_answers_close)",
        "test(=connection::terminal_close_delivery_tests::stalled_cleanup_does_not_hold_socket_open)",
        "test(=connection::terminal_close_delivery_tests::bootstrap_drain_holds_socket_for_client_close_reply)",
        "test(=connection::terminal_close_delivery_tests::stalled_expiry_barrier_does_not_hold_socket_open)",
        "test(=connection::terminal_close_delivery_tests::blocked_frame_handler_does_not_hold_socket_open)",
        "test(=connection::terminal_close_delivery_tests::refused_bootstrap_holds_connection_slot_until_socket_ends)",
        "test(=connection::terminal_close_delivery_tests::stalled_bootstrap_holds_connection_slot)",
        "test(=connection::terminal_close_delivery_tests::bootstrap_admits_from_the_cached_community_answer)",
        "test(=connection::terminal_close_delivery_tests::peer_wait_replies_to_client_initiated_close)",
        "test(=handlers::auth::tests::b2_pre_cancelled_connection_never_becomes_authenticated)",
        "test(=handlers::auth::tests::fi_ban_check_error_emits_terminal_authorization_unavailable)",
        "test(=handlers::auth::tests::fi_root_authorization_denied_rows_emit_identical_frames)",
        "test(=handlers::auth::tests::fi_invalid_nip42_proof_emits_terminal_evidence_rejected)",
        "test(=handlers::auth::tests::handle_auth_pairing_mismatch_runs_full_root_denial_path)",
        "test(=handlers::auth::tests::shadow_root_auth_records_pairing_denial)",
        "test(=handlers::auth::tests::shadow_root_auth_records_unavailable_owner_read)",
        "test(=handlers::ingest::postgres_tests::relay_admin_ban_maps_to_blocked_auth_failure)",
        "test(=handlers::ingest::postgres_tests::relay_admin_internal_maps_to_error_not_client_fault)",
        "test(=handlers::auth::tests::shadow_root_invalid_nip42_retires_without_record)",
        "test(=handlers::auth::tests::nip42_denial_class_separates_internal_failure_from_bad_evidence)",
        "test(=handlers::event::tests::fanout_access::owner_only_kinds_keep_only_the_owner)",
        "test(=handlers::event::tests::pubsub_fanout::pubsub_owner_only_kinds_reach_only_the_owner)",
        "test(=handlers::event::tests::pubsub_fanout::dispatch_owner_only_kinds_reach_only_the_owner)",
        "test(=handlers::req::tests::p1a_huddle_liveness_req_barrier_expiry_blocks_query_and_emission)",
        "test(=state::tests::f3_cancellation_during_check_terminates_socket_without_waiting_for_check)",
        "test(=state::tests::on_not_run_runs_once_on_each_deny_arm_and_never_on_admit)",
        "test(=state::tests::conn_manager_disconnect_nip_fi_ignores_unproven_connection)",
        "test(=state::tests::conn_manager_disconnect_nip_fi_is_issuer_scoped)",
        "test(=state::tests::conn_manager_disconnect_nip_fi_sets_authorization_denied_reason)",
        "test(=state::tests::nip_fi_disconnect_audio_is_issuer_scoped)",
        "test(=state::tests::nip_fi_owner_disconnect_closes_owned_agent_sockets_on_root_and_audio)",
        "test(=state::tests::nip_fi_disconnect_closes_proven_audio_socket_and_sends_policy_close_reason)",
        "test(=state::tests::nip_fi_disconnect_closes_target_audio_only_and_preserves_collocated_peer)",
        "test(=state::tests::nip_fi_disconnect_does_not_close_different_pubkey_audio_socket)",
        "test(=state::tests::nip_fi_disconnect_does_not_close_unproven_audio_socket)",
        "test(=state::tests::community_disconnect_then_nip_fi_keeps_community_deleted_reason)",
        "test(=state::tests::enforce_end_is_recorded_only_by_the_winning_terminal)",
        "test(=state::tests::disconnect_community_wins_reason_losing_nip_fi_does_not_enqueue_frame)",
        "test(=state::tests::manager_disconnect_sets_reason_enqueues_frame_then_cancels)",
        "test(/^api::nip_fi::/)",
      ].join(" + "),
    ],
  },
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "-E",
      "test(/invite_preview/)",
    ],
  },
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "-E",
      "test(/^invite_token::/) + test(/^config::invite_cutoff_tests::/)",
    ],
  },
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "-E",
      "test(/^concurrency_limits::tests::/) + test(/^feature_flags::tests::/) + test(=handlers::auth::tests::auth_cap_sheds_a_verified_login_with_a_bare_close)",
    ],
  },
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "-E",
      "test(=state::tests::admitted_index_follows_every_key_change) + test(/^shared_reads::tests::/) + test(/^membership_cache::tests::/) + test(/^community_active::tests::/) + test(=state::tests::concurrent_open_channel_misses_share_the_in_flight_read) + test(/^state::tests::member_channel_invalidation_tests::/) + test(=state::tests::a_member_snapshot_ages_from_its_read_not_its_insert) + test(=state::tests::overlapping_member_reads_cache_the_first_to_finish) + test(=state::tests::an_open_read_that_straddles_an_invalidation_is_not_cached) + test(=state::tests::concurrent_restriction_and_host_misses_share_the_in_flight_read) + test(=tenant::tests::app_state_resolver_serves_fresh_cached_hosts_only) + test(=tenant::tests::an_overlong_host_fails_closed_before_the_resolver) + test(=state::tests::disconnect_drops_the_cached_restriction_row)",
    ],
  },
  {
    args: [
      "nextest",
      "run",
      "-p",
      "buzz-relay",
      "--lib",
      "-E",
      "test(=mesh_boot::tests::demo_echo_retains_pending_receive_across_housekeeping_ticks)",
    ],
  },
  {
    args: ["nextest", "run", "-p", "buzz-relay", "--test", "boot_lifecycle"],
  },
];

// Cargo fallback filters for buzz-relay when cargo-nextest is not installed.
// Mirrors the relay stanzas in scripts/run-tests.sh.
const BUZZ_RELAY_CARGO_PREFIXES = [
  "api::media_access::tests::",
  "handlers::media_access::tests::",
  "subscription::tests::",
  "handlers::multinode_fanout::tests::",
  "api::admin::",
  "access::tests::",
  "api::git::transport::access_log_tests::",
  "push_runtime::",
  "operator_listener::tests::",
  "handlers::channel_authz::",
  "handlers::moderation_authz::",
  "handlers::side_effects::tests::",
  "handlers::coalesced_refresh::tests::",
  "handlers::system_channels::tests::",
  "nip11::tests::",
  "storage_sweep::tests::",
  "nip_fi_core::tests::",
  "nip_fi_http::tests::",
  "nip_fi_policy::tests::",
  "nip_fi_config::tests::",
  "nip_fi_resolver::tests::",
  "readiness::tests::",
  "router::tests::",
  "api::parse_query_tests::",
  "api::db_read_error_tests::",
  "api::profiles::",
  "api::bridge::search_slots::tests::",
  "api::git::transport::off_mode_precedence_tests::",
  "api::git::store::tests::",
  "api::git::store::fault_probe::",
  "api::git::cas_publish::tests::",
  "api::git::transport::track_c_tests::",
  "audio::join::tests::",
  "audio::handler::tests::",
  "nip_fi_gate::tests::",
  "nip_fi_session::tests::",
  "nip_fi_shadow::tests::",
  "nip_fi_shadow_session::tests::",
  "startup_steps::tests::",
  "mesh_boot::tests::",
  "nip_fi_upgrade::",
  "api::nip_fi::",
  "metrics::contract_tests::",
  "metrics::partition_alert_tests::",
  "invite_preview",
  "invite_token::",
  "config::invite_cutoff_tests::",
  "concurrency_limits::tests::",
  "feature_flags::tests::",
];

// Per-crate infra-free unit test specs matching Justfile `test-unit` (and
// `scripts/run-tests.sh unit` when cargo-nextest is unavailable). Crates not
// listed here have no infra-requiring default tests and run directly with `-p`.
const CRATE_SPECS = {
  "buzz-audio-client": {
    nextest: [{ cmd: "cargo", args: ["test", "-p", "buzz-audio-client"] }],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-audio-client"] }],
  },
  "buzz-core": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-core", "--lib"] },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-core", "--lib"] }],
  },
  "buzz-auth": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-auth", "--lib"] },
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-auth",
          "--test",
          "mesh_jwks_proxy",
        ],
      },
      { cmd: "cargo", args: ["test", "-p", "buzz-auth", "--doc"] },
    ],
    cargo: [
      { cmd: "cargo", args: ["test", "-p", "buzz-auth", "--lib"] },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-auth", "--test", "mesh_jwks_proxy"],
      },
      { cmd: "cargo", args: ["test", "-p", "buzz-auth", "--doc"] },
    ],
  },
  "buzz-audit": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-audit", "--lib"] },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-audit", "--lib"] }],
  },
  "buzz-pubsub": {
    nextest: [
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-pubsub",
          "--lib",
          "-E",
          "test(/^conn_control::tests::/) + test(/^connection::tests::/) + test(=tests::queue_full_retain_does_not_wait_and_schedules_reconciliation)",
        ],
      },
    ],
    cargo: [
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-pubsub", "--lib", "conn_control::tests::"],
      },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-pubsub", "--lib", "connection::tests::"],
      },
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-pubsub",
          "--lib",
          "--",
          "--exact",
          "tests::queue_full_retain_does_not_wait_and_schedules_reconciliation",
        ],
      },
    ],
  },
  "buzz-voice": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-voice", "--lib"] },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-voice", "--lib"] }],
  },
  "buzz-sdk": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-sdk", "--lib"] },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-sdk", "--lib"] }],
  },
  "buzz-workflow": {
    nextest: [
      {
        cmd: "cargo",
        args: ["nextest", "run", "-p", "buzz-workflow", "--lib"],
      },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-workflow", "--lib"] }],
  },
  "buzz-deletion": {
    nextest: [
      {
        cmd: "cargo",
        args: ["nextest", "run", "-p", "buzz-deletion", "--lib"],
      },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-deletion", "--lib"] }],
  },
  "buzz-db": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-db", "--lib"] },
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-db",
          "--test",
          "observability_source",
        ],
      },
      { cmd: "cargo", args: ["test", "-p", "buzz-db", "--doc"] },
    ],
    cargo: [
      { cmd: "cargo", args: ["test", "-p", "buzz-db", "--lib"] },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-db", "--test", "observability_source"],
      },
      { cmd: "cargo", args: ["test", "-p", "buzz-db", "--doc"] },
    ],
  },
  "buzz-search": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-search", "--lib"] },
    ],
    cargo: [{ cmd: "cargo", args: ["test", "-p", "buzz-search", "--lib"] }],
  },
  "buzz-media": {
    nextest: [
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-media",
          "--lib",
          "-E",
          "test(=bucket_index::tests::bucket_snapshot_json_round_trip_preserves_community_keys)",
        ],
      },
    ],
    cargo: [
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-media",
          "--lib",
          "bucket_index::tests::bucket_snapshot_json_round_trip_preserves_community_keys",
          "--",
          "--exact",
        ],
      },
    ],
  },
  "buzz-admin": {
    nextest: [
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-admin",
          "-E",
          "test(storage_snapshot) + test(/^communities::tests::/) + test(/^tests::communities_/) + test(=tests::partition_audit_outcomes_keep_their_json_names_and_exit_codes) + test(/^nip_fi::tests::/) + test(/^nip_fi_policy::tests::/) + test(/^user_deletions::tests::/)",
        ],
      },
    ],
    cargo: [
      { cmd: "cargo", args: ["test", "-p", "buzz-admin", "storage_snapshot"] },
      { cmd: "cargo", args: ["test", "-p", "buzz-admin", "communities"] },
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-admin",
          "tests::partition_audit_outcomes_keep_their_json_names_and_exit_codes",
          "--",
          "--exact",
        ],
      },
      { cmd: "cargo", args: ["test", "-p", "buzz-admin", "nip_fi::tests"] },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-admin", "nip_fi_policy::tests"],
      },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-admin", "user_deletions::tests"],
      },
    ],
  },
  "buzz-push-gateway": {
    nextest: [
      { cmd: "cargo", args: ["nextest", "run", "-p", "buzz-push-gateway"] },
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-push-gateway",
          "--features",
          "personal-dev-app-attest",
        ],
      },
    ],
    cargo: [
      { cmd: "cargo", args: ["test", "-p", "buzz-push-gateway"] },
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-push-gateway",
          "--features",
          "personal-dev-app-attest",
        ],
      },
    ],
  },
  "buzz-feature-flags": {
    nextest: [
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-feature-flags",
          "--features",
          "launchdarkly",
        ],
      },
    ],
    cargo: [
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-feature-flags",
          "--features",
          "launchdarkly",
        ],
      },
    ],
  },
  "buzz-relay": {
    nextest: BUZZ_RELAY_NEXTEST_FILTERS.map((entry) => ({
      cmd: "cargo",
      args: entry.args,
    })),
    cargo: [
      ...BUZZ_RELAY_CARGO_PREFIXES.map((prefix) => ({
        cmd: "cargo",
        args: ["test", "-p", "buzz-relay", "--lib", prefix],
      })),
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-relay",
          "--lib",
          "telemetry::tests::",
          "--",
          "--test-threads=1",
        ],
      },
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-relay",
          "--lib",
          "config::tests::",
          "--",
          "--skip",
          "nip_fi_config::",
          "--test-threads=1",
        ],
      },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-relay", "--bin", "buzz-relay"],
      },
      {
        cmd: "cargo",
        args: ["test", "-p", "buzz-relay", "--test", "boot_lifecycle"],
      },
    ],
  },
  "buzz-test-client": {
    nextest: [
      {
        cmd: "cargo",
        args: [
          "nextest",
          "run",
          "-p",
          "buzz-test-client",
          "--lib",
          "-E",
          "test(/^(loadgen|acceptance)::/)",
        ],
      },
    ],
    cargo: [
      {
        cmd: "cargo",
        args: [
          "test",
          "-p",
          "buzz-test-client",
          "--lib",
          "--",
          "loadgen::",
          "acceptance::",
        ],
      },
    ],
  },
};

export function parseWorkspaceMembers(cargoTomlText) {
  const match = cargoTomlText.match(
    /\[workspace\][\s\S]*?members\s*=\s*\[([\s\S]*?)\]/,
  );
  if (!match) return [];
  const members = [];
  for (const [, item] of match[1].matchAll(/"([^"]+)"/g)) {
    members.push(posix.normalize(item).replace(/\/$/, ""));
  }
  return members;
}

export function parsePackageName(cargoTomlText) {
  const pkgMatch = cargoTomlText.match(/\[package\]([\s\S]*?)(?=\n\[|$)/);
  if (!pkgMatch) return null;
  const nameMatch = pkgMatch[1].match(/^\s*name\s*=\s*"([^"]+)"/m);
  return nameMatch ? nameMatch[1] : null;
}

export function changedWorkspaceCrates(files, repoRoot) {
  const rootTomlPath = resolve(repoRoot, "Cargo.toml");
  if (!existsSync(rootTomlPath)) return [];
  const members = parseWorkspaceMembers(readFileSync(rootTomlPath, "utf8"))
    .slice()
    .sort((a, b) => b.length - a.length);

  const crates = new Map();
  for (const rawFile of files) {
    const file = posix.normalize(rawFile);
    const memberDir = members.find(
      (dir) => file === dir || file.startsWith(`${dir}/`),
    );
    if (!memberDir) continue;
    const manifestPath = resolve(repoRoot, memberDir, "Cargo.toml");
    if (!existsSync(manifestPath)) continue;
    const pkgName = parsePackageName(readFileSync(manifestPath, "utf8"));
    if (!pkgName) continue;
    crates.set(pkgName, memberDir);
  }
  return [...crates.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, dir]) => ({ name, dir }));
}

export function commandsForWorkspaceCrate(
  crateName,
  { hasNextest = true } = {},
) {
  const spec = CRATE_SPECS[crateName];
  if (spec) {
    return (hasNextest ? spec.nextest : spec.cargo).map((step) => ({
      ...step,
      cwd: ".",
    }));
  }
  return [
    {
      cmd: "cargo",
      args: hasNextest
        ? ["nextest", "run", "-p", crateName]
        : ["test", "-p", crateName],
      cwd: ".",
    },
  ];
}

export function selectWorkspacePlan({ files, repoRoot, hasNextest = true }) {
  const crates = changedWorkspaceCrates(files, repoRoot);
  if (!crates.length) {
    return {
      crates: [],
      commands: [],
      message:
        "No workspace crate changed; skipping pre-push Rust tests (dependency, config, migration, and schema checks run in CI).",
    };
  }
  const commands = crates.flatMap(({ name }) =>
    commandsForWorkspaceCrate(name, { hasNextest }),
  );
  return {
    crates,
    commands,
    message: `Pre-push Rust tests for changed crates: ${crates.map((c) => c.name).join(", ")}`,
  };
}

const TAURI_PREFIX = "desktop/src-tauri/";
const TAURI_TERMINAL_PREFIX = "desktop/src-tauri/crates/buzz-terminal/";

function isIgnoredTauriFile(file) {
  return file === "desktop/src-tauri/Cargo.lock" || file.endsWith(".md");
}

export function selectTauriPlan({ files, repoRoot }) {
  const tauriFiles = files
    .map((f) => posix.normalize(f))
    .filter((f) => f.startsWith(TAURI_PREFIX) && !isIgnoredTauriFile(f));

  if (!tauriFiles.length) {
    return {
      scope: "none",
      commands: [],
      message:
        "No desktop Tauri Rust sources changed; skipping pre-push Tauri checks (lockfile/docs checks run in CI).",
    };
  }

  const terminalFiles = tauriFiles.filter((f) =>
    f.startsWith(TAURI_TERMINAL_PREFIX),
  );
  const nonTerminalFiles = tauriFiles.filter(
    (f) => !f.startsWith(TAURI_TERMINAL_PREFIX),
  );
  const terminalExists = existsSync(
    resolve(repoRoot, "desktop/src-tauri/crates/buzz-terminal/Cargo.toml"),
  );

  if (terminalFiles.length > 0 && nonTerminalFiles.length === 0) {
    if (!terminalExists) {
      return {
        scope: "none",
        commands: [],
        message:
          "buzz-terminal crate was removed; skipping pre-push Tauri checks.",
      };
    }
    return {
      scope: "buzz-terminal",
      commands: [
        {
          cmd: "cargo",
          args: [
            "clippy",
            "--manifest-path",
            "desktop/src-tauri/Cargo.toml",
            "-p",
            "buzz-terminal",
            "--all-targets",
            "--",
            "-D",
            "warnings",
          ],
          cwd: ".",
        },
        {
          cmd: "cargo",
          args: [
            "test",
            "--manifest-path",
            "desktop/src-tauri/Cargo.toml",
            "-p",
            "buzz-terminal",
          ],
          cwd: ".",
        },
      ],
      message: "Pre-push Tauri checks scoped to buzz-terminal.",
    };
  }

  const workspaceManifestChanged = nonTerminalFiles.includes(
    "desktop/src-tauri/Cargo.toml",
  );
  if (terminalFiles.length === 0 && !workspaceManifestChanged) {
    return {
      scope: "buzz-desktop",
      commands: [
        { cmd: "just", args: ["_ensure-sidecar-stubs"], cwd: "." },
        {
          cmd: "cargo",
          args: [
            "clippy",
            "--manifest-path",
            "desktop/src-tauri/Cargo.toml",
            "-p",
            "buzz-desktop",
            "--all-targets",
            "--",
            "-D",
            "warnings",
          ],
          cwd: ".",
        },
        {
          cmd: "cargo",
          args: [
            "clippy",
            "--manifest-path",
            "desktop/src-tauri/Cargo.toml",
            "-p",
            "buzz-desktop",
            "--all-targets",
            "--features",
            "mesh-llm",
            "--",
            "-D",
            "warnings",
          ],
          cwd: ".",
        },
        {
          cmd: "cargo",
          args: ["test", "-p", "buzz-desktop"],
          cwd: "desktop/src-tauri",
        },
      ],
      message: "Pre-push Tauri checks scoped to buzz-desktop.",
    };
  }

  return {
    scope: "workspace",
    commands: [
      { cmd: "just", args: ["desktop-tauri-clippy"], cwd: "." },
      { cmd: "just", args: ["desktop-tauri-test"], cwd: "." },
    ],
    message: "Pre-push Tauri checks for desktop/src-tauri workspace.",
  };
}

function gitChangedFiles(repoRoot, base, head) {
  const verify = spawnSync("git", ["rev-parse", "--verify", "--quiet", base], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  if (verify.status !== 0) {
    return { missingBase: true, files: [] };
  }
  const out = execFileSync(
    "git",
    ["diff", "--name-only", "-z", `${base}...${head}`],
    {
      cwd: repoRoot,
      encoding: "utf8",
    },
  );
  return {
    missingBase: false,
    files: out.split("\0").filter(Boolean),
  };
}

function hasCargoNextest() {
  if (process.env.BUZZ_USE_CARGO_TEST === "1") return false;
  const res = spawnSync("cargo-nextest", ["--version"], { stdio: "ignore" });
  return res.status === 0;
}

function parseCliArgs(argv) {
  const mode = argv[0];
  let dryRun = process.env.BUZZ_PRE_PUSH_DRY_RUN === "1";
  let base = "origin/main";
  let head = "HEAD";
  let rootOverride = null;
  let filesJson = null;

  for (let i = 1; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--dry-run") dryRun = true;
    else if (arg === "--base") base = argv[++i];
    else if (arg === "--head") head = argv[++i];
    else if (arg === "--root") rootOverride = argv[++i];
    else if (arg === "--files-json") filesJson = argv[++i];
    else throw new Error(`Unknown option: ${arg}`);
  }
  return { mode, dryRun, base, head, rootOverride, filesJson };
}

function formatCommand(step) {
  const prefix = step.cwd && step.cwd !== "." ? `(cd ${step.cwd} && ` : "";
  const suffix = step.cwd && step.cwd !== "." ? ")" : "";
  const parts = [step.cmd, ...step.args].map((part) =>
    /[\s'"]/u.test(part) ? JSON.stringify(part) : part,
  );
  return `${prefix}${parts.join(" ")}${suffix}`;
}

export function main(argv = process.argv.slice(2)) {
  const { mode, dryRun, base, head, rootOverride, filesJson } =
    parseCliArgs(argv);
  if (mode !== "workspace" && mode !== "tauri") {
    console.error(
      "Usage: pre-push-rust.mjs <workspace|tauri> [--dry-run] [--base <ref>] [--head <ref>]",
    );
    process.exit(2);
  }

  const repoRoot = rootOverride
    ? resolve(rootOverride)
    : execFileSync("git", ["rev-parse", "--show-toplevel"], {
        encoding: "utf8",
      }).trim();

  let files;
  if (filesJson !== null) {
    files = JSON.parse(filesJson);
  } else {
    const diff = gitChangedFiles(repoRoot, base, head);
    if (diff.missingBase) {
      console.log(
        `No local ${base}; skipping pre-push Rust tests (CI runs the full suite).`,
      );
      return 0;
    }
    files = diff.files;
  }

  const plan =
    mode === "workspace"
      ? selectWorkspacePlan({ files, repoRoot, hasNextest: hasCargoNextest() })
      : selectTauriPlan({ files, repoRoot });

  console.log(plan.message);
  for (const step of plan.commands) {
    if (dryRun) {
      console.log(`[dry-run] ${formatCommand(step)}`);
      continue;
    }
    const res = spawnSync(step.cmd, step.args, {
      cwd: resolve(repoRoot, step.cwd || "."),
      stdio: "inherit",
    });
    if (res.error) {
      console.error(res.error.message);
      return 1;
    }
    if ((res.status ?? 1) !== 0) {
      return res.status ?? 1;
    }
  }
  return 0;
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))
) {
  process.exit(main());
}
