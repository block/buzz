#!/bin/bash
set -euo pipefail

# Match desktop's URL-scoped git credential configuration without installing a
# helper globally (which would make it answer for unrelated remotes).
if [[ -n "${BUZZ_RELAY_URL:-}" ]]; then
    relay_http_url="${BUZZ_RELAY_URL/#ws:/http:}"
    relay_http_url="${relay_http_url/#wss:/https:}"
    relay_http_url="${relay_http_url%/}"
    git config --global "credential.${relay_http_url}/git.helper" \
        /usr/local/bin/git-credential-nostr
    git config --global "credential.${relay_http_url}/git.useHttpPath" true
fi

# The harness must receive Kubernetes' termination signal directly.
exec buzz-acp "$@"
