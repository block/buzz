#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# stub error() for sourced helper
error() { echo "error: $*" >&2; }
# shellcheck source=scripts/lib/docker_preflight.sh
source "${ROOT}/scripts/lib/docker_preflight.sh"

assert_eq() {
  local got="$1" want="$2" label="$3"
  if [[ "$got" != "$want" ]]; then
    echo "FAIL $label: got='$got' want='$want'" >&2
    exit 1
  fi
  echo "ok $label"
}

assert_eq "$(docker_preflight_classify '' 0)" ok "success"
assert_eq "$(docker_preflight_classify 'permission denied while trying to connect' 1)" permission "permission"
assert_eq "$(docker_preflight_classify 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?' 1)" not_running "not_running"
assert_eq "$(docker_preflight_classify 'dial unix /var/run/docker.sock: connect: permission denied' 1)" permission "dial_unix_permission"
assert_eq "$(docker_preflight_classify 'something weird happened' 1)" other "other"
echo "all docker_preflight tests passed"
