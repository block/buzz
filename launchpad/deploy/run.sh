#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
COMPOSE_ENV="${REPO_ROOT}/deploy/compose/.env"
CANONICAL_RUNNER="${REPO_ROOT}/deploy/compose/run.sh"
MINIMUM_COMPOSE_VERSION="2.24.4"

fail() {
  echo "Launchpad deployment check failed: $*" >&2
  exit 1
}

version_at_least() {
  local actual=$1
  local minimum=$2
  local actual_major actual_minor actual_patch
  local minimum_major minimum_minor minimum_patch

  IFS=. read -r actual_major actual_minor actual_patch <<<"${actual}"
  IFS=. read -r minimum_major minimum_minor minimum_patch <<<"${minimum}"

  ((actual_major > minimum_major)) ||
    ((actual_major == minimum_major && actual_minor > minimum_minor)) ||
    ((actual_major == minimum_major && actual_minor == minimum_minor && actual_patch >= minimum_patch))
}

usage() {
  cat <<'MSG'
Usage: ./launchpad/deploy/run.sh <command>

Runs Launchpad image and Compose preflight checks, then delegates to the
canonical deploy/compose/run.sh implementation.

Use `check` to validate without changing services. All canonical commands are
accepted. A floating Launchpad tag is rejected unless the operator explicitly
sets BUZZ_ALLOW_FLOATING_IMAGE=true for development or testing.
MSG
}

case "${1:-help}" in
  help|-h|--help)
    usage
    exit 0
    ;;
esac

command -v docker >/dev/null 2>&1 || fail "Docker is not installed or is not on PATH."
docker compose version >/dev/null 2>&1 || fail "Docker Compose V2 is not available."

compose_version=$(docker compose version --short)
compose_version=${compose_version#v}
compose_version=${compose_version%%-*}
[[ "${compose_version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  fail "Could not parse Docker Compose version '${compose_version}'."
version_at_least "${compose_version}" "${MINIMUM_COMPOSE_VERSION}" ||
  fail "Docker Compose ${MINIMUM_COMPOSE_VERSION} or newer is required; found ${compose_version}."

[[ -f "${COMPOSE_ENV}" ]] ||
  fail "Missing deploy/compose/.env. Copy deploy/compose/.env.example and configure it first."

image_count=$(awk '
  /^[[:space:]]*#/ { next }
  /^[[:space:]]*BUZZ_IMAGE[[:space:]]*=/ { count++ }
  END { print count + 0 }
' "${COMPOSE_ENV}")
[[ "${image_count}" == "1" ]] ||
  fail "deploy/compose/.env must contain exactly one BUZZ_IMAGE assignment."

image=$(awk '
  /^[[:space:]]*#/ { next }
  /^[[:space:]]*BUZZ_IMAGE[[:space:]]*=/ {
    value = $0
    sub(/^[^=]*=/, "", value)
    sub(/^[[:space:]]+/, "", value)
    sub(/[[:space:]]+$/, "", value)
    print value
  }
' "${COMPOSE_ENV}")

if [[ "${image}" == \"*\" && "${image}" == *\" ]]; then
  image=${image:1:${#image}-2}
elif [[ "${image}" == \'*\' && "${image}" == *\' ]]; then
  image=${image:1:${#image}-2}
fi

[[ -n "${image}" ]] || fail "BUZZ_IMAGE is empty."
lower_image=$(printf '%s' "${image}" | tr '[:upper:]' '[:lower:]')

case "${lower_image}" in
  ghcr.io/block/buzz|ghcr.io/block/buzz:*|ghcr.io/block/buzz@*)
    fail "Upstream Block image '${image}' is forbidden for Launchpad deployment."
    ;;
esac

case "${lower_image}" in
  ghcr.io/launchpad-26/buzz:*|ghcr.io/launchpad-26/buzz@*) ;;
  *) fail "BUZZ_IMAGE must select ghcr.io/launchpad-26/buzz; found '${image}'." ;;
esac

immutable=false
if [[ "${lower_image}" =~ ^ghcr\.io/launchpad-26/buzz@sha256:[0-9a-f]{64}$ ]] ||
  [[ "${lower_image}" =~ ^ghcr\.io/launchpad-26/buzz:sha-[0-9a-f]{40}$ ]]; then
  immutable=true
fi

if [[ "${immutable}" != "true" ]]; then
  echo "WARNING: BUZZ_IMAGE is not a digest or full commit-SHA tag: ${image}" >&2
  [[ "${BUZZ_ALLOW_FLOATING_IMAGE:-false}" == "true" ]] ||
    fail "Floating images are rejected. Set BUZZ_ALLOW_FLOATING_IMAGE=true only for intentional development/testing use."
fi

echo "Launchpad relay image: ${image}"
echo "Docker Compose version: ${compose_version}"

export BUZZ_IMAGE="${image}"
if [[ "$1" == "check" ]]; then
  "${CANONICAL_RUNNER}" config >/dev/null
  echo "Launchpad deployment configuration is valid."
  exit 0
fi

exec "${CANONICAL_RUNNER}" "$@"
