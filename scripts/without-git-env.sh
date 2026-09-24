#!/usr/bin/env bash
# Git exports repository-local variables (GIT_DIR, GIT_INDEX_FILE, ...) to
# hooks. Pre-push lanes run test suites whose fixtures spawn git in temp dirs;
# with those variables inherited, fixtures operate on this checkout instead.
# Transport, credential, and TLS settings (GIT_SSH_COMMAND, GIT_ASKPASS, ...)
# are left intact.
set -euo pipefail
# shellcheck disable=SC2046
unset $(git rev-parse --local-env-vars)
exec "$@"
