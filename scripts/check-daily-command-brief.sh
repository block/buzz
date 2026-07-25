#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cargo_bin="${CARGO:-cargo}"
pnpm_bin="${PNPM:-pnpm}"
xcodebuild_bin="${XCODEBUILD:-xcodebuild}"
live_mode=false

usage() {
  printf 'usage: %s [--live]\n' "$(basename "$0")" >&2
}

for argument in "$@"; do
  case "${argument}" in
    --live) live_mode=true ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage
      exit 64
      ;;
  esac
done

run_check() {
  local label="$1"
  shift
  printf '[daily-command-brief] %s\n' "${label}"
  "$@"
}

require_loopback_url() {
  local label="$1"
  local value="$2"
  local path_pattern="$3"
  if [[ ! "${value}" =~ ^http://127\.0\.0\.1:([1-9][0-9]{0,4})${path_pattern}$ ]]; then
    printf '[daily-command-brief] live configuration denied: %s must be literal IPv4 loopback\n' \
      "${label}" >&2
    return 1
  fi
  local port="${BASH_REMATCH[1]}"
  if ((port > 65535)); then
    printf '[daily-command-brief] live configuration denied: %s port is invalid\n' \
      "${label}" >&2
    return 1
  fi
}

run_live_probes() {
  local lm_studio_url="${BUZZ_DAILY_BRIEF_LM_STUDIO_URL:-}"
  local lm_studio_model="${BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL:-}"
  local memory_url="${BUZZ_DAILY_BRIEF_MEMORY_URL:-}"
  local rag_url="${BUZZ_DAILY_BRIEF_RAG_URL:-}"
  local live_driver="${BUZZ_DAILY_BRIEF_LIVE_DRIVER:-}"

  if [[ -z "${lm_studio_url}" || -z "${lm_studio_model}" ||
        -z "${memory_url}" || -z "${rag_url}" || -z "${live_driver}" ]]; then
    printf '%s\n' \
      '[daily-command-brief] live configuration denied: set all explicit loopback URLs, model, and live driver' >&2
    return 1
  fi
  require_loopback_url "LM Studio URL" "${lm_studio_url}" ""
  require_loopback_url "Memory MCP URL" "${memory_url}" "/mcp/"
  require_loopback_url "RAG MCP URL" "${rag_url}" "/mcp/"
  if [[ "${lm_studio_model}" != "${lm_studio_model//$'\n'/}" ||
        "${lm_studio_model}" != "${lm_studio_model//$'\r'/}" ||
        ${#lm_studio_model} -gt 256 ]]; then
    printf '%s\n' \
      '[daily-command-brief] live configuration denied: model identifier is invalid' >&2
    return 1
  fi
  if [[ "${live_driver}" != /* || ! -f "${live_driver}" || ! -x "${live_driver}" ||
        -L "${live_driver}" ]]; then
    printf '%s\n' \
      '[daily-command-brief] live configuration denied: live driver must be an explicit executable regular file' >&2
    return 1
  fi

  run_check \
    "LM Studio native structured-output smoke on the configured loopback model" \
    bash scripts/check-lmstudio-native.sh \
      --base-url "${lm_studio_url}" \
      --model "${lm_studio_model}" \
      --smoke \
      --reasoning off
  run_check \
    "Operator-approved signed-app offline, egress, resource, and history exercise" \
    env \
      BUZZ_DAILY_BRIEF_LM_STUDIO_URL="${lm_studio_url}" \
      BUZZ_DAILY_BRIEF_MEMORY_URL="${memory_url}" \
      BUZZ_DAILY_BRIEF_RAG_URL="${rag_url}" \
      "${live_driver}"
}

cd "${repo_root}"

run_check \
  "LM Studio literal-loopback, structured-tool, pseudo-tool, proxy, and response-bound fixtures" \
  bash scripts/tests/check-lmstudio-native-test.sh
run_check \
  "Native adviser executor positive and negative fixtures" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_brief::lmstudio_tests -- --test-threads=1
run_check \
  "Frozen RAG, Memory, Apple, prompt-injection, and source-policy fixtures" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_brief::sources_tests -- --test-threads=1
run_check \
  "Five-specialist and tool-free Chief of Staff fixture orchestration" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_brief::orchestrator_tests -- --test-threads=1
run_check \
  "Schedule idempotency, wake catch-up, manual-run separation, and recovery" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_brief::schedule_tests -- --test-threads=1
run_check \
  "Encrypted local spool, history reload, and publication retry" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_brief::store_tests -- --test-threads=1
run_check \
  "NIP-44 owner-only signed lifecycle audit" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    command_brief::audit_tests -- --test-threads=1
run_check \
  "Owner-locked Tauri command boundary" \
  "${cargo_bin}" test --manifest-path desktop/src-tauri/Cargo.toml \
    commands::command_brief::tests -- --test-threads=1
run_check \
  "Command Brief wire, encryption, tags, and bounded payload contract" \
  "${cargo_bin}" test -p buzz-core command_brief --lib
run_check \
  "Relay owner-only REQ, COUNT, ID, search, and ingest gates" \
  "${cargo_bin}" test -p buzz-relay command_brief
run_check \
  "Clean-profile encrypted backup and exact restore fixtures" \
  bash scripts/tests/local-workspace-backup-test.sh
run_check \
  "Command Console contracts, hooks, and immutable brief presentation" \
  "${pnpm_bin}" --dir desktop test
run_check \
  "Read-only Apple helper fixtures" \
  env DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
    "${xcodebuild_bin}" test \
      -project desktop/apple-inputs/BuzzAppleInputs.xcodeproj \
      -scheme BuzzAppleInputs \
      -destination platform=macOS \
      CODE_SIGNING_ALLOWED=NO

printf '[daily-command-brief] all hermetic Phase 4 Daily Command Brief checks passed\n'
if "${live_mode}"; then
  run_live_probes
  printf '[daily-command-brief] explicit live probes passed\n'
else
  printf '%s\n' \
    '[daily-command-brief] live probes skipped: pass --live with explicit literal-loopback configuration'
fi
