#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_file="${repo_root}/docker-compose.oss-e2e.yml"
project="buzz-oss-e2e"
state_dir="${TMPDIR:-/tmp}/buzz-oss-e2e-${UID}"
relay_bin="${repo_root}/target/debug/buzz-relay"
git_helper="${repo_root}/target/debug/git-credential-nostr"
relay_identity="ws://127.0.0.1:3301"
tenant_host="127.0.0.1:3301"
scenario_ids=(A01 D01 D02 D03 D04 L01 L02 L03 R01 O501 P01)
completed_scenarios=()
failed_scenario=""

export DATABASE_URL="postgres://buzz:buzz_oss_e2e@127.0.0.1:5546/buzz" # sadscan:disable np.postgres.1
export BUZZ_TEST_DATABASE_URL="${DATABASE_URL}"
export REDIS_URL="redis://127.0.0.1:6546"
export BUZZ_S3_ENDPOINT="http://127.0.0.1:9546"
export BUZZ_S3_ACCESS_KEY="buzz_oss_e2e"
export BUZZ_S3_SECRET_KEY="buzz_oss_e2e_synthetic_secret"
export BUZZ_S3_BUCKET="buzz-media"
export BUZZ_S3_REGION="us-east-1"
export BUZZ_S3_ADDRESSING_STYLE="path"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
export OSS_E2E_RELAY_IDENTITY="${relay_identity}"
export OSS_E2E_RELAY_A_WS="ws://127.0.0.1:3301"
export OSS_E2E_RELAY_B_WS="ws://127.0.0.1:3302"
export OSS_E2E_RELAY_A_HTTP="http://127.0.0.1:3301"
export OSS_E2E_RELAY_B_HTTP="http://127.0.0.1:3302"
export OSS_E2E_RELAY_A_METRICS="http://127.0.0.1:9301/metrics"
export OSS_E2E_RELAY_B_METRICS="http://127.0.0.1:9302/metrics"
export OSS_E2E_TENANT_HOST="${tenant_host}"
export OSS_E2E_RELAY_A_LOG="${state_dir}/relay-a.log"
export OSS_E2E_RELAY_B_LOG="${state_dir}/relay-b.log"
export OSS_E2E_RESTART_STATE="${state_dir}/restart-state.json"
export OSS_E2E_SUMMARY="${state_dir}/summary.json"
export GIT_CREDENTIAL_NOSTR_BIN="${git_helper}"

compose() {
  docker compose --project-name "${project}" --file "${compose_file}" "$@"
}

cargo_test() {
  "${repo_root}/bin/cargo" test "$@"
}

pid_file() {
  printf '%s/%s.pid' "${state_dir}" "${1:?relay name is required}"
}

relay_log() {
  printf '%s/%s.log' "${state_dir}" "${1:?relay name is required}"
}

relay_command() {
  ps -p "${1:?pid is required}" -o command=
}

stop_relay() {
  local relay_name="${1:?relay name is required}"
  local file
  file="$(pid_file "${relay_name}")"
  if [[ ! -f "${file}" ]]; then
    return 0
  fi
  local relay_pid
  relay_pid="$(<"${file}")"
  if [[ ! "${relay_pid}" =~ ^[0-9]+$ ]]; then
    printf 'refusing ambiguous %s pid file: %s\n' "${relay_name}" "${file}" >&2
    return 1
  fi
  if ! kill -0 "${relay_pid}" 2>/dev/null; then
    rm -f "${file}"
    return 0
  fi
  local command_line
  command_line="$(relay_command "${relay_pid}")"
  if [[ "${command_line}" != *"${relay_bin}"* ]]; then
    printf 'refusing to stop unowned pid %s for %s: %s\n' \
      "${relay_pid}" "${relay_name}" "${command_line}" >&2
    return 1
  fi
  kill "${relay_pid}"
  local attempt
  for attempt in $(seq 1 40); do
    if ! kill -0 "${relay_pid}" 2>/dev/null; then
      rm -f "${file}"
      return 0
    fi
    sleep 0.25
  done
  printf 'owned %s pid %s did not stop after SIGTERM\n' "${relay_name}" "${relay_pid}" >&2
  return 1
}

wait_readiness() {
  local relay_name="${1:?relay name is required}"
  local health_port="${2:?health port is required}"
  local file
  file="$(pid_file "${relay_name}")"
  local attempt
  for attempt in $(seq 1 90); do
    local relay_pid
    relay_pid="$(<"${file}")"
    if ! kill -0 "${relay_pid}" 2>/dev/null; then
      printf '%s exited before readiness\n' "${relay_name}" >&2
      tail -n 120 "$(relay_log "${relay_name}")" >&2
      return 1
    fi
    local status_code
    status_code="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${health_port}/_readiness" || true)"
    if [[ "${status_code}" == "200" ]]; then
      return 0
    fi
    sleep 1
  done
  printf '%s did not become ready\n' "${relay_name}" >&2
  tail -n 120 "$(relay_log "${relay_name}")" >&2
  return 1
}

start_relay() {
  local relay_name="${1:?relay name is required}"
  local bind_port="${2:?bind port is required}"
  local health_port="${3:?health port is required}"
  local metrics_port="${4:?metrics port is required}"
  local auto_migrate="${5:?auto migrate flag is required}"
  local file
  file="$(pid_file "${relay_name}")"
  if [[ -f "${file}" ]]; then
    printf '%s already has a pid file; run stop before start\n' "${relay_name}" >&2
    return 1
  fi
  nohup env \
    DATABASE_URL="${DATABASE_URL}" \
    REDIS_URL="${REDIS_URL}" \
    RELAY_URL="${relay_identity}" \
    BUZZ_BIND_ADDR="127.0.0.1:${bind_port}" \
    BUZZ_HEALTH_PORT="${health_port}" \
    BUZZ_METRICS_PORT="${metrics_port}" \
    BUZZ_AUTO_MIGRATE="${auto_migrate}" \
    BUZZ_DB_POOL_SIZE=8 \
    BUZZ_REDIS_POOL_SIZE=8 \
    BUZZ_REQUIRE_AUTH_TOKEN=false \
    BUZZ_REQUIRE_RELAY_MEMBERSHIP=false \
    BUZZ_HUDDLE_AUDIO_AVAILABLE=true \
    BUZZ_USAGE_METRICS_PER_COMMUNITY=off \
    BUZZ_MESH=off \
    BUZZ_S3_ENDPOINT="${BUZZ_S3_ENDPOINT}" \
    BUZZ_S3_ACCESS_KEY="${BUZZ_S3_ACCESS_KEY}" \
    BUZZ_S3_SECRET_KEY="${BUZZ_S3_SECRET_KEY}" \
    BUZZ_S3_BUCKET="${BUZZ_S3_BUCKET}" \
    BUZZ_S3_REGION="${BUZZ_S3_REGION}" \
    BUZZ_S3_ADDRESSING_STYLE="${BUZZ_S3_ADDRESSING_STYLE}" \
    BUZZ_MEDIA_BASE_URL="http://127.0.0.1:${bind_port}/media" \
    RUST_LOG=buzz_relay=info \
    "${relay_bin}" >>"$(relay_log "${relay_name}")" 2>&1 &
  local relay_pid=$!
  printf '%s\n' "${relay_pid}" >"${file}"
  wait_readiness "${relay_name}" "${health_port}"
}

build_binaries() {
  "${repo_root}/bin/cargo" build -p buzz-relay -p git-credential-nostr
  [[ -x "${relay_bin}" ]]
  [[ -x "${git_helper}" ]]
}

setup_dependencies() {
  compose up --detach --wait postgres redis minio
  compose run --rm minio-init
}

setup() {
  mkdir -p "${state_dir}"
  setup_dependencies
  build_binaries
  stop_relay relay-b
  stop_relay relay-a
  : >"${OSS_E2E_RELAY_A_LOG}"
  : >"${OSS_E2E_RELAY_B_LOG}"
  rm -f "${OSS_E2E_RESTART_STATE}" "${OSS_E2E_SUMMARY}"
  start_relay relay-a 3301 8301 9301 true
  start_relay relay-b 3302 8302 9302 false
  status
}

restart_relay_b() {
  stop_relay relay-b
  start_relay relay-b 3302 8302 9302 false
}

status_relay() {
  local relay_name="${1:?relay name is required}"
  local file
  file="$(pid_file "${relay_name}")"
  if [[ ! -f "${file}" ]]; then
    printf '%s: stopped\n' "${relay_name}"
    return 0
  fi
  local relay_pid
  relay_pid="$(<"${file}")"
  if kill -0 "${relay_pid}" 2>/dev/null; then
    printf '%s: running pid=%s\n' "${relay_name}" "${relay_pid}"
  else
    printf '%s: stale pid=%s\n' "${relay_name}" "${relay_pid}"
    return 1
  fi
}

status() {
  compose ps
  status_relay relay-a
  status_relay relay-b
}

stop() {
  stop_relay relay-b
  stop_relay relay-a
  compose down --remove-orphans
}

cleanup_after_run() {
  local command_rc=$?
  trap - EXIT
  stop_relay relay-b || command_rc=$?
  stop_relay relay-a || command_rc=$?
  compose down --remove-orphans || command_rc=$?
  exit "${command_rc}"
}

write_summary() {
  local overall="${1:?overall result is required}"
  local temporary="${OSS_E2E_SUMMARY}.tmp"
  local head
  head="$(git -C "${repo_root}" rev-parse HEAD)"
  {
    printf '{\n'
    printf '  "schema": "buzz.v1.oss-only-e2e-summary.v1",\n'
    printf '  "source_head": "%s",\n' "${head}"
    printf '  "overall": "%s",\n' "${overall}"
    printf '  "executed_scenario_count": %s,\n' "${#completed_scenarios[@]}"
    printf '  "scenarios": ['
    local separator=""
    local scenario_id
    for scenario_id in "${completed_scenarios[@]}"; do
      printf '%s{"id":"%s","status":"PASS"}' "${separator}" "${scenario_id}"
      separator=","
    done
    if [[ -n "${failed_scenario}" ]]; then
      printf '%s{"id":"%s","status":"FAIL"}' "${separator}" "${failed_scenario}"
    fi
    printf ']\n'
    printf '}\n'
  } >"${temporary}"
  mv "${temporary}" "${OSS_E2E_SUMMARY}"
}

run_live_topology() {
  if ! cargo_test -p buzz-relay --test oss_only_e2e \
    live_two_relay_clients_and_migrations -- --ignored --exact --nocapture; then
    failed_scenario="TOPOLOGY_PRE_RESTART"
    return 1
  fi
  completed_scenarios+=(M01 A01 D01 D02 H01 G01 AU01 P01)
  restart_relay_b
  if ! cargo_test -p buzz-relay --test oss_only_e2e \
    restarted_relay_restores_persisted_event -- --ignored --exact --nocapture; then
    failed_scenario="R01"
    return 1
  fi
  completed_scenarios+=(R01)
}

run_scenario() {
  local scenario_id="${1:?scenario ID is required}"
  case "${scenario_id}" in
    A01)
      cargo_test -p buzz-auth current_allow_returns_request_scoped_snapshot
      ;;
    D01)
      cargo_test -p buzz-auth provider_unavailability_never_falls_back_to_allow
      ;;
    D02)
      cargo_test -p buzz-relay duplicate_or_unknown_domains_fail_closed
      ;;
    D03)
      cargo_test -p buzz-auth stale_and_future_provider_decisions_deny
      ;;
    D04)
      cargo_test -p buzz-auth mismatched_embedded_proof_domain_fails_before_authority_io
      ;;
    L01)
      cargo_test -p buzz-db principal_can_be_disabled_before_first_enrollment -- --ignored
      ;;
    L02)
      cargo_test -p buzz-auth direct_lease_carries_binding_and_earliest_application_expiry
      ;;
    L03)
      cargo_test -p buzz-relay projection_worker_retries_after_restart_and_fans_out_canonical_withdrawal -- --ignored
      ;;
    R01)
      cargo_test -p buzz-relay restart_bootstraps_full_state_before_readiness
      ;;
    O501)
      cargo_test -p buzz-relay --test o5_operator_postgres &&
      cargo_test -p buzz-db postgres_o5_outbox_rollback_delivery_restore_and_capacity_are_non_vacuous
      ;;
    P01)
      cargo_test -p buzz-relay --test o5_operator_surface planted_canaries_never_cross_response_logs_or_metrics &&
      cargo_test -p buzz-db postgres_operator_lifecycle_is_atomic_idempotent_and_serialized
      ;;
    *)
      printf 'unknown scenario: %s\nvalid scenarios: TOPOLOGY %s\n' \
        "${scenario_id}" "${scenario_ids[*]}" >&2
      return 64
      ;;
  esac
}

run_all() {
  setup
  trap cleanup_after_run EXIT
  if ! run_live_topology; then
    write_summary FAIL
    return 1
  fi
  local scenario_id
  for scenario_id in "${scenario_ids[@]}"; do
    if ! run_scenario "${scenario_id}"; then
      failed_scenario="${scenario_id}"
      write_summary FAIL
      return 1
    fi
    completed_scenarios+=("${scenario_id}")
  done
  write_summary PASS
  printf '%s\n' "${OSS_E2E_SUMMARY}"
}

run_one() {
  local scenario_id="${1:?scenario ID is required}"
  setup
  trap cleanup_after_run EXIT
  if [[ "${scenario_id}" == "TOPOLOGY" ]]; then
    if ! run_live_topology; then
      write_summary FAIL
      return 1
    fi
  elif ! run_scenario "${scenario_id}"; then
    failed_scenario="${scenario_id}"
    write_summary FAIL
    return 1
  else
    completed_scenarios+=("${scenario_id}")
  fi
  write_summary PASS
  printf '%s\n' "${OSS_E2E_SUMMARY}"
}

usage() {
  cat <<'USAGE'
usage: scripts/oss-e2e.sh setup|run|reset|stop|status|scenario ID

setup starts PostgreSQL, Redis, MinIO, and two real stock relay processes.
run drives the live topology plus all focused contract cards and then cleans up.
scenario TOPOLOGY runs only the live migration/client/restart matrix.

All services, credentials, fixtures, and identifiers are local and synthetic.
The stock relay binary registers no O5 operator routes.
USAGE
}

command_name="${1:-}"
case "${command_name}" in
  setup)
    setup
    ;;
  run)
    run_all
    ;;
  reset)
    stop_relay relay-b
    stop_relay relay-a
    compose down --volumes --remove-orphans
    setup
    ;;
  stop)
    stop
    ;;
  status)
    status
    ;;
  scenario)
    run_one "${2:-}"
    ;;
  *)
    usage >&2
    exit 64
    ;;
esac
