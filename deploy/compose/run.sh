#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

COMPOSE_FILES=(-f compose.yml)
if [[ "${BUZZ_COMPOSE_TLS:-false}" == "true" ]]; then
  COMPOSE_FILES+=(-f compose.caddy.yml)
fi
if [[ "${BUZZ_COMPOSE_DEV:-false}" == "true" ]]; then
  COMPOSE_FILES+=(-f compose.dev.yml)
fi

compose() {
  docker compose --env-file .env "${COMPOSE_FILES[@]}" "$@"
}

require_env() {
  if [[ ! -f .env ]]; then
    cat >&2 <<'MSG'
Missing deploy/compose/.env.

Copy .env.example to .env and replace every CHANGE_ME value, or run the bootstrap
script once it lands. Do not start production with generated secrets missing.
MSG
    exit 1
  fi
  if grep -Eq '^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*=.*CHANGE_ME' .env; then
    cat >&2 <<'MSG'
deploy/compose/.env still contains CHANGE_ME placeholders.
Generate stable secrets first; these values must not rotate on restart.
MSG
    exit 1
  fi
}

backup_hint() {
  cat <<'MSG'
Back up these before upgrades and on a regular schedule:

- deploy/compose/.env, especially BUZZ_RELAY_PRIVATE_KEY, DB/Redis/Typesense/S3 secrets, and BUZZ_GIT_HOOK_HMAC_SECRET
- The owner private key if bootstrap generated one for RELAY_OWNER_PUBKEY
- Postgres data (prefer pg_dump or a quiesced volume snapshot)
- MinIO/S3 bucket contents for media and git objects
- buzz-git-data volume (BUZZ_GIT_REPO_PATH=/data/git)
- Caddy data/config volumes if using compose.caddy.yml

Keep Postgres + object/git state snapshots from the same maintenance window.
MSG
}

case "${1:-help}" in
  start|up)
    require_env
    compose up -d --wait
    ;;
  stop|down)
    compose down
    ;;
  restart)
    require_env
    compose up -d --wait --force-recreate relay
    ;;
  pull)
    require_env
    compose pull
    ;;
  upgrade)
    require_env
    compose pull
    compose up -d --wait
    backup_hint
    ;;
  logs)
    shift || true
    compose logs -f "${@:-relay}"
    ;;
  status|ps)
    compose ps
    ;;
  config)
    require_env
    compose config
    ;;
  backup-hint)
    backup_hint
    ;;
  add-member)
    docker compose exec relay /usr/local/bin/buzz-admin add-member --pubkey "${2:?Usage: ./run.sh add-member <npub-or-hex> [--role member|admin]}" "${@:3}"
    ;;
  remove-member)
    docker compose exec relay /usr/local/bin/buzz-admin remove-member --pubkey "${2:?Usage: ./run.sh remove-member <npub-or-hex> [--role member|admin]}" "${@:3}"
    ;;
  list-members)
    docker compose exec relay /usr/local/bin/buzz-admin list-members
    ;;
  help|-h|--help)
    cat <<'MSG'
Usage: ./run.sh <command>

Commands:
  start         Start Buzz with docker compose up -d --wait
  stop          Stop containers without deleting volumes
  restart       Recreate the relay after env/image changes
  pull          Pull configured images
  upgrade       Pull and restart, then print backup reminders
  logs [svc]    Follow logs (default: relay)
  status        Show compose service status
  config        Render merged compose config
  backup-hint   Print the production backup checklist

  add-member <npub-or-hex> [--role member|admin]
                Add a relay member (default role: member)
  remove-member <npub-or-hex> [--role member|admin]
                Remove a relay member
  list-members  List all relay members

  Note: when adding multiple members in a loop, add `sleep 1` between
  invocations to avoid same-second timestamp collisions in the kind:13534
  roster event. Do not use parallel adds (e.g. xargs -P).

Environment switches:
  BUZZ_COMPOSE_TLS=true   Include compose.caddy.yml for automatic HTTPS
  BUZZ_COMPOSE_DEV=true   Include compose.dev.yml for local admin ports/tools
MSG
    ;;
  *)
    echo "Unknown command: $1" >&2
    echo "Run ./run.sh help" >&2
    exit 1
    ;;
esac
