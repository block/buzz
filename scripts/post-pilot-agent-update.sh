#!/usr/bin/env bash
# Post a fixed-format agent update into Steve's Buzz pilot channel, with optional Slack mirroring.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/post-pilot-agent-update.sh \
    --status <started|blocked|needs-steve|changed|handoff|done> \
    --task-title <title> \
    --summary <summary> \
    --next-owner <owner> \
    [--reply-to <event-id>] \
    [--changed <path>]...
EOF
}

json_escape() {
  local value="${1:-}"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  printf '%s' "${value}"
}

redact_sensitive() {
  local text="${1:-}"
  if [[ -n "${BUZZ_PRIVATE_KEY:-}" ]]; then
    text="${text//${BUZZ_PRIVATE_KEY}/<redacted-private-key>}"
  fi
  if [[ -n "${BUZZ_PILOT_SLACK_WEBHOOK_URL:-}" ]]; then
    text="${text//${BUZZ_PILOT_SLACK_WEBHOOK_URL}/<redacted-slack-webhook>}"
  fi
  printf '%s' "${text}"
}

slack_text_is_safe() {
  local value="${1:-}"
  [[ "${value}" =~ nsec1[0-9a-zA-Z]+ ]] && return 1
  [[ "${value}" =~ xox[baprs]- ]] && return 1
  [[ "${value}" =~ https://hooks\.slack\.com/services ]] && return 1
  [[ "${value}" =~ BUZZ_(PILOT_PROOF_)?PRIVATE_KEY= ]] && return 1
  [[ "${value}" =~ -----BEGIN[[:space:]]+.*PRIVATE[[:space:]]+KEY----- ]] && return 1
  [[ "${value}" =~ https?://(localhost|127\.0\.0\.1)(:[0-9]+)? ]] && return 1
  [[ "${value}" =~ (^|[^0-9A-Fa-f])(localhost|127\.0\.0\.1):[0-9]+([^0-9A-Fa-f]|$) ]] && return 1
  [[ "${value}" =~ (^|[^0-9A-Fa-f])[0-9A-Fa-f]{64}([^0-9A-Fa-f]|$) ]] && return 1
  return 0
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_env() {
  local name="$1"
  [[ -n "${!name:-}" ]] || die "${name} must be set"
}

validate_status() {
  case "$1" in
    started|blocked|needs-steve|changed|handoff|done) ;;
    *)
      die "unsupported --status: $1"
      ;;
  esac
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

status=""
task_title=""
summary=""
next_owner=""
reply_to=""
declare -a changed_paths=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --status)
      [[ $# -ge 2 ]] || die "--status requires a value"
      status="$2"
      shift 2
      ;;
    --task-title)
      [[ $# -ge 2 ]] || die "--task-title requires a value"
      task_title="$2"
      shift 2
      ;;
    --summary)
      [[ $# -ge 2 ]] || die "--summary requires a value"
      summary="$2"
      shift 2
      ;;
    --next-owner)
      [[ $# -ge 2 ]] || die "--next-owner requires a value"
      next_owner="$2"
      shift 2
      ;;
    --reply-to)
      [[ $# -ge 2 ]] || die "--reply-to requires a value"
      reply_to="$2"
      shift 2
      ;;
    --changed)
      [[ $# -ge 2 ]] || die "--changed requires a value"
      changed_paths+=("$2")
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "${status}" ]] || die "--status is required"
[[ -n "${task_title}" ]] || die "--task-title is required"
[[ -n "${summary}" ]] || die "--summary is required"
[[ -n "${next_owner}" ]] || die "--next-owner is required"

validate_status "${status}"
require_env "BUZZ_RELAY_URL"
require_env "BUZZ_PRIVATE_KEY"
require_env "BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID"

if [[ -n "${reply_to}" ]]; then
  case "${status}" in
    blocked|needs-steve|changed|handoff|done) ;;
    *)
      die "reply updates must use one of: blocked, needs-steve, changed, handoff, done"
      ;;
  esac
fi

channel_id="${BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID}"
if [[ -n "${BUZZ_PILOT_CHANNEL_ID_OVERRIDE:-}" ]]; then
  channel_id="${BUZZ_PILOT_CHANNEL_ID_OVERRIDE}"
  echo "warning: using test-only channel override ${channel_id}" >&2
fi

BUZZ_CLI="${BUZZ_PILOT_CLI:-}"
if [[ -z "${BUZZ_CLI}" ]]; then
  if [[ -x "${REPO_ROOT}/target/debug/buzz" ]]; then
    BUZZ_CLI="${REPO_ROOT}/target/debug/buzz"
  elif [[ -x "${REPO_ROOT}/target/release/buzz" ]]; then
    BUZZ_CLI="${REPO_ROOT}/target/release/buzz"
  elif [[ -x "${REPO_ROOT}/.hermit/rust/bin/buzz" ]]; then
    BUZZ_CLI="${REPO_ROOT}/.hermit/rust/bin/buzz"
  else
    die "Buzz CLI not found; set BUZZ_PILOT_CLI"
  fi
fi
[[ -x "${BUZZ_CLI}" ]] || die "Buzz CLI is not executable: ${BUZZ_CLI}"

needs_steve="no"
if [[ "${status}" == "blocked" || "${status}" == "needs-steve" ]]; then
  needs_steve="yes"
fi

message_body=$(cat <<EOF
[${status}] ${task_title}
Summary: ${summary}
Next Owner: ${next_owner}
Needs Steve: ${needs_steve}
EOF
)

if [[ ${#changed_paths[@]} -gt 0 ]]; then
  for changed_path in "${changed_paths[@]}"; do
    message_body+=$'\n'"Changed: ${changed_path}"
  done
fi

buzz_cmd=("${BUZZ_CLI}" "messages" "send" "--channel" "${channel_id}" "--content" "-")
if [[ -n "${reply_to}" ]]; then
  buzz_cmd+=("--reply-to" "${reply_to}")
fi

set +e
buzz_output="$(
  printf '%s\n' "${message_body}" | \
    BUZZ_RELAY_URL="${BUZZ_RELAY_URL}" \
    BUZZ_PRIVATE_KEY="${BUZZ_PRIVATE_KEY}" \
    "${buzz_cmd[@]}" 2>&1
)"
buzz_status=$?
set -e

if [[ "${buzz_status}" -ne 0 ]]; then
  echo "Buzz post failed: $(redact_sensitive "${buzz_output}")" >&2
  exit "${buzz_status}"
fi

event_id="$(printf '%s\n' "${buzz_output}" | sed -n 's/.*"event_id":"\([0-9a-f]\{64\}\)".*/\1/p' | head -n 1)"
[[ -n "${event_id}" ]] || die "unable to parse event_id from Buzz response"

buzz_reference="buzz://message?channel=${channel_id}&id=${event_id}"
slack_status="skipped"
slack_error=""

if [[ -n "${BUZZ_PILOT_SLACK_WEBHOOK_URL:-}" ]]; then
  slack_safe=1
  for slack_value in "${task_title}" "${next_owner}"; do
    if ! slack_text_is_safe "${slack_value}"; then
      slack_safe=0
    fi
  done
  if [[ ${#changed_paths[@]} -gt 0 ]]; then
    for slack_value in "${changed_paths[@]}"; do
      if ! slack_text_is_safe "${slack_value}"; then
        slack_safe=0
      fi
    done
  fi

  if [[ "${slack_safe}" -eq 0 ]]; then
    slack_status="skipped_unsafe"
  else
    slack_text=$(cat <<EOF
[${status}] ${task_title}
Needs Steve: ${needs_steve}
Buzz: ${buzz_reference}
EOF
)
    if [[ ${#changed_paths[@]} -gt 0 ]]; then
      for changed_path in "${changed_paths[@]}"; do
        slack_text+=$'\n'"Changed: ${changed_path}"
      done
    fi

    slack_payload="{\"text\":\"$(json_escape "${slack_text}")\"}"

    set +e
    slack_output="$(
      curl \
        --silent \
        --show-error \
        --fail \
        -X POST \
        -H 'Content-Type: application/json' \
        --data "${slack_payload}" \
        "${BUZZ_PILOT_SLACK_WEBHOOK_URL}" 2>&1
    )"
    slack_exit=$?
    set -e

    if [[ "${slack_exit}" -eq 0 ]]; then
      slack_status="mirrored"
    else
      slack_status="failed"
      slack_error="$(redact_sensitive "${slack_output}")"
    fi
  fi
fi

printf '{'
printf '"event_id":"%s",' "$(json_escape "${event_id}")"
printf '"channel_id":"%s",' "$(json_escape "${channel_id}")"
printf '"status":"%s",' "$(json_escape "${status}")"
printf '"buzz_reference":"%s",' "$(json_escape "${buzz_reference}")"
printf '"slack_status":"%s"' "$(json_escape "${slack_status}")"
if [[ -n "${slack_error}" ]]; then
  printf ',"slack_error":"%s"' "$(json_escape "${slack_error}")"
fi
printf '}\n'
