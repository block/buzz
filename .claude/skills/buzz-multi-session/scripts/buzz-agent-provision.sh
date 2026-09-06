#!/usr/bin/env bash
# buzz-agent-provision.sh — give a non-Claude-Code agent an identity on the relay.
#
#   buzz-agent-provision.sh <name> [--channel <name>] [--command <harness>]
#                                  [--owner <pubkey>] [--auth-tag <json>]
#                                  [--force]
#
# buzz-acp runs goose, codex and other harnesses against a relay. What each of
# them needs to get there is identical, and it is exactly what buzz-connect.sh
# already automates for a Claude Code session: a keypair, relay membership, a
# published name, and channel membership. Doing that by hand is what keeps a
# hosted agent stuck.
#
# Three differences from a session identity, and they are the whole reason this
# is a separate command rather than a flag on buzz-connect.sh:
#
#   1. The name is given, not resolved. There is no /rename to follow and no
#      transcript to read, so the identity is NOT bound to any session id — a
#      later /rename in the terminal that provisioned it must not drag the
#      agent's identity along with it.
#   2. No watcher. The harness is a daemon with its own event loop; a Monitor
#      would be a second reader of the same channel.
#   3. The output is an env block for a Dockerfile, a fly secret or a systemd
#      unit — not a session that starts talking.
#
# The private key is never printed. Its file path is, which is all anyone needs.
set -uo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

usage() {
  die "usage: $0 <name> [--channel <name>] [--command <harness>]
                        [--owner <pubkey>] [--auth-tag <json>] [--force]

  <name>            the agent's name. It becomes ~/.buzz/sessions/<name>.env and
                    the display name on the relay.
  --channel <name>  join or create this channel and admit the agent to it
  --command <c>     the buzz-acp harness this identity is for; recorded and
                    printed in the env block
  --owner <pubkey>  the human who owns this agent (see 'ownership' below)
  --auth-tag <json> a NIP-OA auth tag to record for the agent
  --force           re-publish the profile even if it has not changed"
}

NAME=""
CHANNEL_ARG=""
COMMAND_ARG=""
OWNER_ARG=""
AUTH_TAG_ARG=""
FORCE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --channel)  CHANNEL_ARG="${2:-}"; shift ;;
    --command)  COMMAND_ARG="${2:-}"; shift ;;
    --owner)    OWNER_ARG="${2:-}"; shift ;;
    --auth-tag) AUTH_TAG_ARG="${2:-}"; shift ;;
    --force)    FORCE=1 ;;
    -h|--help)  usage ;;
    -*)         usage ;;
    *)          [ -n "$NAME" ] && usage; NAME="$1" ;;
  esac
  shift
done
[ -n "$NAME" ] || usage

check_config_perms
require_buzz

# --- 1. identity --------------------------------------------------------------
# CLAUDE_CODE_SESSION_ID is deliberately dropped. buzz-session.sh binds an
# identity to the session that created it so a /rename can follow it; an agent
# identity must never be adopted that way, or renaming this terminal would rename
# a daemon's key out from under it.
IDENT=$(env -u CLAUDE_CODE_SESSION_ID "$HERE/buzz-session.sh" resolve "$NAME") || exit 1
AGENT_NAME=$(printf '%s' "$IDENT" | cut -f1)
AGENT_DISPLAY=$(printf '%s' "$IDENT" | cut -f2)
PUBKEY=$(printf '%s' "$IDENT" | cut -f3)
IDFILE=$(printf '%s' "$IDENT" | cut -f4)
load_identity "$IDFILE" || die "could not load identity $IDFILE"
RELAY="${BUZZ_RELAY_URL:-http://localhost:3000}"
export RUST_LOG="${RUST_LOG:-error}"

# --- 1b. ownership inputs, validated before anything is published -------------
if [ -n "$OWNER_ARG" ]; then
  case "$OWNER_ARG" in *[!0-9a-f]*)
    die "--owner must be a 64-character lowercase hex pubkey" ;;
  esac
  [ "${#OWNER_ARG}" = 64 ] \
    || die "--owner must be a 64-character lowercase hex pubkey (got ${#OWNER_ARG})"
fi

if [ -n "$AUTH_TAG_ARG" ]; then
  TAG_OWNER=$(auth_tag_owner "$AUTH_TAG_ARG") || die \
"--auth-tag is not a NIP-OA attestation. It must be exactly
  [\"auth\", \"<owner pubkey, 64 hex>\", \"<conditions>\", \"<signature, 128 hex>\"]
Mint one on a machine holding the owner's secret key:
  cargo run --release --example compute_auth_tag -- <owner_secret_hex> $PUBKEY \"\""
  if [ -n "$OWNER_ARG" ] && [ "$OWNER_ARG" != "$TAG_OWNER" ]; then
    die "--owner ($OWNER_ARG) is not the owner in --auth-tag ($TAG_OWNER).
Drop --owner: the tag is the authority, and a mismatch here would publish one
owner while attesting another."
  fi
  # Exported for every relay call below, deliberately. buzz verifies the tag
  # against this identity's pubkey and refuses to run if it does not match, so a
  # bad tag fails here rather than silently at deploy time; the profile publish
  # then carries it onto the agent's kind:0, and the authenticated request is
  # what makes the relay record users.agent_owner_pubkey.
  export BUZZ_AUTH_TAG="$AUTH_TAG_ARG"
fi

# Mark it as an agent so `buzz-connect.sh status --all` does not report it as a
# session that never ran. It is unbound on purpose.
meta_set "$AGENT_NAME" BUZZ_AGENT 1
[ -n "$COMMAND_ARG" ] && meta_set "$AGENT_NAME" BUZZ_AGENT_COMMAND "$COMMAND_ARG"

printf 'agent    : %s\nidentity : %s\npubkey   : %s\nrelay    : %s\n' \
  "$AGENT_DISPLAY" "$AGENT_NAME" "$PUBKEY" "$RELAY"

# --- 2. relay membership ------------------------------------------------------
# With an attestation, whether this key is a DIRECT member decides whether the
# relay will ever record its owner, so find out before doing anything about it.
# The probe has to run without the tag: with it, Member and ViaOwner are the same
# 200 and the client cannot tell them apart.
DIRECT_MEMBER=0
ALLOW_CLAIM=1
if [ -n "$AUTH_TAG_ARG" ]; then
  ALLOW_CLAIM=0
  SAVED_TAG="$BUZZ_AUTH_TAG"
  unset BUZZ_AUTH_TAG
  relay_probe && DIRECT_MEMBER=1
  export BUZZ_AUTH_TAG="$SAVED_TAG"
fi
ensure_relay_membership "$PUBKEY" "$RELAY" "$ALLOW_CLAIM" || exit $?

# --- 3. profile ---------------------------------------------------------------
# No "Claude Code (...)" prefix: this is not a Claude Code session, and a wrong
# prefix in a channel listing is worse than none.
[ "$FORCE" = 1 ] && meta_unset "$AGENT_NAME" BUZZ_PROFILE_NAME
publish_profile "$AGENT_NAME" "$AGENT_DISPLAY"

# --- 4. channel ---------------------------------------------------------------
if [ -n "$CHANNEL_ARG" ]; then
  if ! resolve_channel "$CHANNEL_ARG" 1; then
    note "could not find or create channel '${CHANNEL_NAME:-?}': ${BUZZ_ERR:-(no detail)}"
    exit 2
  fi
  if [ "$CHANNEL_CREATED" = 1 ]; then
    echo "channel  : created '$CHANNEL_NAME' ($CHANNEL)"
  else
    echo "channel  : ${CHANNEL_NAME:-<uuid>} ($CHANNEL)"
    MEMBER=""
    if buzz_run channels members --channel "$CHANNEL"; then
      MEMBER=$(printf '%s' "$BUZZ_OUT" | ME="$PUBKEY" python3 -c '
import json, os, sys
me = os.environ["ME"]
try:
    rows = json.load(sys.stdin)
except Exception:
    rows = []
for row in rows if isinstance(rows, list) else []:
    if isinstance(row, dict) and row.get("pubkey") == me:
        sys.stdout.write("1"); break
')
    fi
    if [ -z "$MEMBER" ] \
       && ! join_channel "$CHANNEL" "${CHANNEL_NAME:-$CHANNEL}" "$PUBKEY"; then
      diagnose_channel "$CHANNEL" "${CHANNEL_NAME:-$CHANNEL}" "$PUBKEY" ""
      exit 4
    fi
  fi
  meta_set "$AGENT_NAME" BUZZ_SESSION_CHANNEL "$CHANNEL"
  meta_set "$AGENT_NAME" BUZZ_SESSION_CHANNEL_NAME "${CHANNEL_NAME:-}"
  meta_set "$AGENT_NAME" BUZZ_SESSION_CHANNEL_RELAY "$(relay_tag)"
fi

# --- 5. ownership -------------------------------------------------------------
agent_ownership_report "$AGENT_NAME" "$PUBKEY" "$OWNER_ARG" "$AUTH_TAG_ARG" \
  "$DIRECT_MEMBER"

# --- 6. the env block ---------------------------------------------------------
agent_env_block "$AGENT_NAME" "$PUBKEY" "$RELAY" "$IDFILE" "$COMMAND_ARG" \
  "${CHANNEL:-}" "${CHANNEL_NAME:-}"
