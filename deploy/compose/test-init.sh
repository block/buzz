#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="$(mktemp -d /tmp/buzz-compose-init-test.XXXXXXXXXX)"

cleanup() {
  case "${WORK_DIR}" in
    /tmp/buzz-compose-init-test.*) rm -rf -- "${WORK_DIR}" ;;
  esac
}
trap cleanup EXIT

make_case() {
  local name="$1"
  local case_dir="${WORK_DIR}/${name}"
  install -d "$case_dir"
  cp "${SCRIPT_DIR}/run.sh" "${SCRIPT_DIR}/.env.example" "$case_dir/"
  printf '%s\n' "$case_dir"
}

owner=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

success_dir="$(make_case success)"
(
  cd "$success_dir"
  ./run.sh init Buzz.Example.Com "$owner" >stdout 2>stderr
  [[ ! -s stderr ]]
  grep -qx 'Created deploy/compose/.env with mode 600; review and back it up before starting Buzz' stdout
  [[ "$(stat -c '%a' .env)" == 600 ]]
  grep -qx 'BUZZ_DOMAIN=buzz.example.com' .env
  grep -qx 'RELAY_URL=wss://buzz.example.com' .env
  grep -qx 'BUZZ_MEDIA_BASE_URL=https://buzz.example.com/media' .env
  grep -qx "RELAY_OWNER_PUBKEY=$owner" .env
  if grep -Eq '^[A-Za-z_][A-Za-z0-9_]*=.*CHANGE_ME' .env; then
    printf 'init left a CHANGE_ME assignment in .env\n' >&2
    exit 1
  fi
  grep -Eq '^BUZZ_RELAY_PRIVATE_KEY=[0-9a-f]{64}$' .env
  grep -Eq '^BUZZ_GIT_HOOK_HMAC_SECRET=[0-9a-f]{64}$' .env
  grep -Eq '^POSTGRES_PASSWORD=[0-9a-f]{64}$' .env
  grep -Eq '^REDIS_PASSWORD=[0-9a-f]{64}$' .env
  grep -Eq '^BUZZ_S3_ACCESS_KEY=buzz[0-9a-f]{24}$' .env
  grep -Eq '^BUZZ_S3_SECRET_KEY=[0-9a-f]{64}$' .env

  before="$(sha256sum .env)"
  if ./run.sh init buzz.example.com "$owner" >/dev/null 2>overwrite.stderr; then
    printf 'init overwrote an existing .env\n' >&2
    exit 1
  fi
  grep -qx 'Refusing to overwrite deploy/compose/.env' overwrite.stderr
  [[ "$(sha256sum .env)" == "$before" ]]
)

invalid_domain_dir="$(make_case invalid-domain)"
(
  cd "$invalid_domain_dir"
  if ./run.sh init 'https://buzz.example.com' "$owner" >/dev/null 2>stderr; then
    printf 'init accepted an invalid domain\n' >&2
    exit 1
  fi
  grep -qx 'Domain must be a valid DNS name such as buzz.example.com' stderr
  [[ ! -e .env ]]
)

invalid_owner_dir="$(make_case invalid-owner)"
(
  cd "$invalid_owner_dir"
  if ./run.sh init buzz.example.com not-a-pubkey >/dev/null 2>stderr; then
    printf 'init accepted an invalid owner public key\n' >&2
    exit 1
  fi
  grep -qx 'Owner public key must be 64 hexadecimal characters' stderr
  [[ ! -e .env ]]
)

symlink_dir="$(make_case symlink)"
(
  cd "$symlink_dir"
  touch target
  ln -s target .env
  if ./run.sh init buzz.example.com "$owner" >/dev/null 2>stderr; then
    printf 'init replaced an .env symlink\n' >&2
    exit 1
  fi
  grep -qx 'Refusing to overwrite deploy/compose/.env' stderr
  [[ -L .env ]]
  [[ ! -s target ]]
)

missing_env_dir="$(make_case missing-env)"
(
  cd "$missing_env_dir"
  if ./run.sh config >/dev/null 2>stderr; then
    printf 'config accepted a missing .env\n' >&2
    exit 1
  fi
  grep -Fqx 'Run ./run.sh init <domain> <owner-pubkey-hex>, or install .env.example as a' stderr
  grep -Fqx 'mode-600 .env and replace every CHANGE_ME value manually. Do not start' stderr
)

printf 'Production Compose environment initialization checks passed\n'
