#!/usr/bin/env bash
# Control the Buzz agent services installed in the local lab.

set -euo pipefail

declare -A SERVICE=(
  [relay]=buzz-relay.service
  [ace]=buzz-ace.service
  [last30days]=buzz-last30days.service
  [haute]=buzz-haute.service
)

readonly USAGE="Usage: $(basename "$0") {up|down|restart|status} [all|relay|ace|last30days|haute ...]"

die() {
  echo "Error: $*" >&2
  echo "$USAGE" >&2
  exit 2
}

require_systemd_user_session() {
  if ! systemctl --user show-environment >/dev/null 2>&1; then
    die "no systemd user session is available"
  fi
}

resolve_agents() {
  if (($# == 0)); then
    printf '%s\n' relay ace last30days haute
    return
  fi

  local agent
  for agent in "$@"; do
    if [[ "$agent" == all ]]; then
      printf '%s\n' relay ace last30days haute
    elif [[ -v "SERVICE[$agent]" ]]; then
      printf '%s\n' "$agent"
    else
      die "unknown agent '$agent'"
    fi
  done | awk '!seen[$0]++'
}

service_state() {
  local state
  state=$(systemctl --user is-active "${SERVICE[$1]}" 2>/dev/null || true)
  case "$state" in
    active|activating|deactivating|failed) printf '%s' "$state" ;;
    *) printf '%s' inactive ;;
  esac
}

show_status() {
  local agent state
  printf '%-12s %-24s %s\n' AGENT SERVICE STATE
  for agent in relay ace last30days haute; do
    state=$(service_state "$agent")
    printf '%-12s %-24s %s\n' "$agent" "${SERVICE[$agent]}" "$state"
  done
}

change_state() {
  local action=$1
  shift
  local -a agents
  mapfile -t agents < <(resolve_agents "$@")

  local -a ordered
  if [[ "$action" == stop ]]; then
    ordered=(haute last30days ace relay)
  else
    ordered=(relay ace last30days haute)
  fi

  local agent
  for agent in "${ordered[@]}"; do
    if printf '%s\n' "${agents[@]}" | rg -qx "$agent"; then
      echo "$action ${SERVICE[$agent]}"
      systemctl --user "$action" "${SERVICE[$agent]}"
    fi
  done
}

main() {
  (($# >= 1)) || die "missing action"
  require_systemd_user_session

  local action=$1
  shift
  case "$action" in
    up|start) change_state start "$@" ;;
    down|stop) change_state stop "$@" ;;
    restart)
      change_state stop "$@"
      change_state start "$@"
      ;;
    status) (($# == 0)) || die "status does not take agent names"; show_status ;;
    *) die "unknown action '$action'" ;;
  esac
}

main "$@"
