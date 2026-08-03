# lib.sh — shared helpers for the buzz-multi-session scripts.
# Sourced, never executed. Every function here is safe to call more than once.
# shellcheck shell=bash

# Directory holding these scripts, regardless of the caller's cwd.
BUZZ_SKILL_SCRIPTS="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export BUZZ_SKILL_SCRIPTS

SESSION_DIR="${BUZZ_SESSION_DIR:-$HOME/.buzz/sessions}"
CONFIG_FILE="${BUZZ_CONFIG:-$HOME/.buzz/config}"

note() { printf '%s\n' "$*" >&2; }
die()  { printf '%s\n' "$*" >&2; exit 1; }

# --- configuration -----------------------------------------------------------
# ~/.buzz/config is shared by every session on the machine. It is parsed, never
# sourced — it holds an invite code, so it must not be able to run code.
# Format: KEY=value, one per line, '#' comments, blank lines ignored.
config_get() {
  local key="$1" line
  [ -f "$CONFIG_FILE" ] || return 1
  line=$(grep -E "^[[:space:]]*${key}[[:space:]]*=" "$CONFIG_FILE" 2>/dev/null | tail -n 1) || return 1
  [ -n "$line" ] || return 1
  line=${line#*=}
  # trim surrounding whitespace and one layer of quotes
  line=$(printf '%s' "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
                                   -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'\$/\1/")
  [ -n "$line" ] || return 1
  printf '%s' "$line"
}

# setting KEY [DEFAULT] — environment wins, then ~/.buzz/config, then default.
setting() {
  local key="$1" default="${2:-}" val
  val="${!key:-}"
  [ -n "$val" ] && { printf '%s' "$val"; return 0; }
  val=$(config_get "$key") && { printf '%s' "$val"; return 0; }
  printf '%s' "$default"
}

# config_set KEY VALUE — record a value for every session on this machine.
# This is what stops the second session creating a duplicate channel: a private
# channel is invisible to a non-member, so "find it by name" cannot work and the
# creator has to publish the UUID somewhere the others read.
config_set() {
  local key="$1" val="$2" tmp
  mkdir -p "$(dirname "$CONFIG_FILE")"
  [ -f "$CONFIG_FILE" ] || { : > "$CONFIG_FILE"; chmod 600 "$CONFIG_FILE"; }
  tmp=$(mktemp -t buzz-config) || return 1
  chmod 600 "$tmp"
  KEY="$key" VAL="$val" python3 -c '
import os, re, sys
key, val = os.environ["KEY"], os.environ["VAL"]
out, seen = [], False
for line in open(sys.argv[1]):
    if re.match(r"^\s*%s\s*=" % re.escape(key), line):
        if seen:
            continue
        out.append("%s=%s\n" % (key, val)); seen = True
    else:
        out.append(line)
if not seen:
    out.append("%s=%s\n" % (key, val))
sys.stdout.write("".join(out))
' "$CONFIG_FILE" > "$tmp" && mv "$tmp" "$CONFIG_FILE" && chmod 600 "$CONFIG_FILE"
}

# Warn once if the config file is world/group readable — it can hold an invite
# code, which is a bearer token for relay membership.
check_config_perms() {
  [ -f "$CONFIG_FILE" ] || return 0
  local mode
  # shellcheck disable=SC2012  # fixed filename; ls -l mode chars are portable
  mode=$(ls -l "$CONFIG_FILE" 2>/dev/null | cut -c5-10)
  case "$mode" in
    ---------|'') ;;
    *[rwx]*) note "warning: $CONFIG_FILE is readable by others — chmod 600 it (it can hold an invite code)" ;;
  esac
}

# --- binaries ----------------------------------------------------------------
# PATH first, then a release/debug build in the enclosing checkout, which is the
# common case for someone hacking on block/buzz.
resolve_bin() {
  local name="$1" var="$2" found root cand
  found="${!var:-}"
  if [ -n "$found" ]; then printf '%s' "$found"; return 0; fi
  # ~/.buzz/config outranks PATH deliberately. A configured path is a decision;
  # PATH is ambient, and on a machine with Buzz Desktop installed it resolves to
  # the app's bundled CLI, which lags the relay's features. That shadowing is
  # invisible and produces a wrong-but-plausible failure: the script reports the
  # feature missing, which is true of the binary it picked and false of the one
  # the user configured. Config also lets a session coordinating worktrees of
  # some *other* repo find binaries that live in a Buzz checkout.
  if found=$(config_get "$var") && [ -n "$found" ]; then
    printf '%s' "$found"; return 0
  fi
  if found=$(command -v "$name" 2>/dev/null); then printf '%s' "$found"; return 0; fi
  if root=$(git rev-parse --show-toplevel 2>/dev/null); then
    for cand in "$root/target/release/$name" "$root/target/debug/$name"; do
      [ -x "$cand" ] && { printf '%s' "$cand"; return 0; }
    done
  fi
  return 1
}

require_buzz() {
  BUZZ=$(resolve_bin buzz BUZZ_BIN) || die \
"buzz not found.
  Fix: cargo build --release -p buzz-cli   (then it is picked up from target/release)
  Or:  export BUZZ_BIN=/path/to/buzz"
  export BUZZ
}

# --- session identities ------------------------------------------------------
# Two files per identity, and the split is deliberate:
#
#   <name>.env   sourced into the environment, so it holds ONLY shell-safe
#                values: two 64-char hex keys and a validated relay URL.
#   <name>.meta  never sourced, only grepped. Everything derived from free text
#                lives here — a /rename title is arbitrary user input, and a
#                title like "x$(rm -rf ~)" in a sourced file would execute.
identity_file() { printf '%s/%s.env' "$SESSION_DIR" "$1"; }
meta_file()     { printf '%s/%s.meta' "$SESSION_DIR" "$1"; }

# field FILE KEY — read one value without exposing the rest of the file.
identity_field() {
  [ -f "$1" ] || return 1
  local v
  v=$(grep -E "^$2=" "$1" 2>/dev/null | tail -n 1 | cut -d= -f2-) || return 1
  [ -n "$v" ] || return 1
  printf '%s' "$v"
}

# meta_get NAME KEY / meta_set NAME KEY VALUE — the free-text sidecar.
meta_get() { identity_field "$(meta_file "$1")" "$2"; }

meta_set() {
  local f tmp
  f=$(meta_file "$1")
  mkdir -p "$SESSION_DIR"
  [ -f "$f" ] || { : > "$f"; chmod 600 "$f"; }
  tmp=$(mktemp -t buzz-meta) || return 1
  chmod 600 "$tmp"
  KEY="$2" VAL="$3" python3 -c '
import os, sys
key, val = os.environ["KEY"], os.environ["VAL"].replace("\n", " ")
out, seen = [], False
for line in open(sys.argv[1]):
    if line.startswith(key + "="):
        if seen:
            continue
        out.append("%s=%s\n" % (key, val)); seen = True
    else:
        out.append(line)
if not seen:
    out.append("%s=%s\n" % (key, val))
sys.stdout.write("".join(out))
' "$f" > "$tmp" && mv "$tmp" "$f" && chmod 600 "$f"
}

# meta_unset NAME KEY — drop a key. Used when a session leaves a room: a stale
# pin would keep buzz-msg.sh posting into a channel this session is no longer in.
meta_unset() {
  local f tmp
  f=$(meta_file "$1")
  [ -f "$f" ] || return 0
  tmp=$(mktemp -t buzz-meta) || return 1
  chmod 600 "$tmp"
  KEY="$2" python3 -c '
import os, sys
key = os.environ["KEY"] + "="
with open(sys.argv[1]) as fh:
    sys.stdout.write("".join(l for l in fh if not l.startswith(key)))
' "$f" > "$tmp" && mv "$tmp" "$f" && chmod 600 "$f"
}

# The identity belongs to the session, not to its current name: /rename changes
# the name, so look the identity up by the session id it was minted under.
identity_for_session() {
  local sid="${CLAUDE_CODE_SESSION_ID:-}" f
  [ -n "$sid" ] || return 1
  [ -d "$SESSION_DIR" ] || return 1
  for f in "$SESSION_DIR"/*.meta; do
    [ -f "$f" ] || continue
    if [ "$(identity_field "$f" BUZZ_SESSION_ID 2>/dev/null)" = "$sid" ]; then
      printf '%s' "$(basename "$f" .meta)"; return 0
    fi
  done
  return 1
}

# Load an identity into the environment. Scripts call this themselves — nobody
# is ever told to `set -a; . file; set +a` by hand.
#
# The file is validated before it is sourced. Anything other than a comment or
# one of the three known keys with a conservative value is refused rather than
# executed: a sourced file is code, and this one is written by a script.
load_identity() {
  local f="$1" env_relay="${BUZZ_RELAY_URL:-}" bad
  [ -f "$f" ] || return 1
  bad=$(grep -vE '^[[:space:]]*(#.*)?$|^BUZZ_(PRIVATE_KEY|PUBKEY)=[0-9a-fA-F]{64}$|^BUZZ_RELAY_URL=[A-Za-z0-9:/._~%+-]+$' "$f")
  if [ -n "$bad" ]; then
    note "refusing to source $f — unexpected content. Delete it and re-run buzz-connect.sh."
    return 1
  fi
  set -a
  # shellcheck disable=SC1090  # runtime path, one identity file per session
  . "$f"
  set +a
  # An explicit BUZZ_RELAY_URL in the caller's environment outranks the value
  # recorded when the key was minted.
  [ -n "$env_relay" ] && export BUZZ_RELAY_URL="$env_relay"
  return 0
}

# --- running the CLI ---------------------------------------------------------
# buzz_run <args...> — stdout in BUZZ_OUT, stderr in BUZZ_ERR, status returned.
# shellcheck disable=SC2034  # both are read by the scripts that source this
BUZZ_OUT=""
# shellcheck disable=SC2034
BUZZ_ERR=""
buzz_run() {
  local err rc
  err=$(mktemp -t buzz-err) || return 127
  BUZZ_OUT=$("$BUZZ" "$@" 2>"$err"); rc=$?
  # shellcheck disable=SC2034  # read by the scripts that source this
  BUZZ_ERR=$(cat "$err" 2>/dev/null)
  rm -f "$err"
  return $rc
}

# --- the coordination channel ------------------------------------------------
is_uuid() {
  case "$1" in
    [0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]-[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]-*-*-*) return 0 ;;
    *) return 1 ;;
  esac
}

default_channel_name() { setting BUZZ_COORD_CHANNEL_NAME "agent-coordination"; }

# channel_cache_key NAME — the ~/.buzz/config key that caches this channel's
# UUID. One slot per channel name, because a single BUZZ_COORD_CHANNEL cannot
# hold two rooms: opening a second dedicated channel overwrote the first, and
# the sessions still pointing at the old UUID went quiet with no error at all.
# The default name keeps the historical key, so existing configs keep working.
channel_cache_key() {
  local name="$1" slug
  [ "$name" = "$(default_channel_name)" ] && { printf 'BUZZ_COORD_CHANNEL'; return 0; }
  slug=$(printf '%s' "$name" | LC_ALL=C tr '[:lower:]' '[:upper:]' | LC_ALL=C tr -c 'A-Z0-9' '_')
  printf 'BUZZ_CHANNEL_%s' "${slug:0:48}"
}

# resolve_channel <uuid-or-name-or-empty> <create:0|1>
# Sets CHANNEL, CHANNEL_NAME, CHANNEL_CREATED, CHANNEL_KEY.
# Needs $BUZZ and a loaded identity; reads $SESSION_NAME if it is set.
CHANNEL=""
CHANNEL_NAME=""
# shellcheck disable=SC2034  # read by the scripts that source this
CHANNEL_CREATED=0
# shellcheck disable=SC2034
CHANNEL_KEY=""
resolve_channel() {
  local want="${1:-}" create="${2:-0}" cached pin
  CHANNEL=""; CHANNEL_CREATED=0; CHANNEL_KEY=""
  if [ -n "$want" ] && is_uuid "$want"; then
    CHANNEL="$want"; CHANNEL_NAME=""
    return 0
  fi

  if [ -z "$want" ]; then
    # A UUID in the environment is an explicit decision; it outranks everything.
    if is_uuid "${BUZZ_COORD_CHANNEL:-}"; then
      CHANNEL="$BUZZ_COORD_CHANNEL"
      CHANNEL_NAME=$(default_channel_name)
      CHANNEL_KEY=BUZZ_COORD_CHANNEL
      return 0
    fi
    # Then the room this session last connected to. Pinned per *session*, not
    # per machine: session A can sit in the default channel while session B
    # works in pp-refactor, and buzz-msg.sh in each posts where that session
    # actually is rather than where the machine's default points.
    if [ -n "${SESSION_NAME:-}" ]; then
      pin=$(meta_get "$SESSION_NAME" BUZZ_SESSION_CHANNEL || printf '')
      if is_uuid "$pin"; then
        CHANNEL="$pin"
        CHANNEL_NAME=$(meta_get "$SESSION_NAME" BUZZ_SESSION_CHANNEL_NAME || printf '')
        [ -n "$CHANNEL_NAME" ] || CHANNEL_NAME=$(default_channel_name)
        CHANNEL_KEY=$(channel_cache_key "$CHANNEL_NAME")
        return 0
      fi
    fi
    want=$(default_channel_name)
  fi

  CHANNEL_NAME="$want"
  CHANNEL_KEY=$(channel_cache_key "$CHANNEL_NAME")
  cached=$(setting "$CHANNEL_KEY" "")
  if is_uuid "$cached"; then CHANNEL="$cached"; return 0; fi

  buzz_run channels list --limit 500 || return 2
  CHANNEL=$(printf '%s' "$BUZZ_OUT" | WANT="$CHANNEL_NAME" python3 -c '
import json, os, sys
want = os.environ["WANT"]
try:
    rows = json.load(sys.stdin)
except Exception:
    rows = []
for row in rows if isinstance(rows, list) else []:
    if isinstance(row, dict) and row.get("name") == want:
        sys.stdout.write(row.get("channel_id") or row.get("id") or "")
        break
')
  if [ -n "$CHANNEL" ]; then config_set "$CHANNEL_KEY" "$CHANNEL"; return 0; fi
  [ "$create" = 1 ] || return 1

  buzz_run channels create --name "$CHANNEL_NAME" --type stream \
    --visibility private --description "Claude Code multi-session coordination" \
    || return 2
  CHANNEL=$(printf '%s' "$BUZZ_OUT" | python3 -c '
import json, sys
try:
    sys.stdout.write(json.load(sys.stdin).get("channel_id") or "")
except Exception:
    pass
')
  [ -n "$CHANNEL" ] || return 2
  # shellcheck disable=SC2034
  CHANNEL_CREATED=1
  # Publish the UUID so the next session on this machine joins this channel
  # instead of creating a second one with the same name that nobody shares.
  config_set "$CHANNEL_KEY" "$CHANNEL"
}

# --- joining a channel someone else owns --------------------------------------
# Relay membership and channel membership are separate gates, and only the
# channel's owner can open the second one. When the owner's key is already on
# this machine — the normal case, because every session here mints its identity
# into ~/.buzz/sessions — asking a human to run `channels add-member` is asking
# them to be a relay for a decision they already made. So use the key.
#
# The safety model is three rules, enforced below and nowhere else:
#   1. Only keys already in $SESSION_DIR. Nothing is minted, fetched or derived.
#   2. Only `channels add-member --role member`, only on the channel being
#      joined. The role is a literal, the channel is the one just resolved.
#   3. Every use is printed: which identity authorised it, and what it ran.
# Relay membership is deliberately NOT in scope — see SKILL.md.

# _as_identity <identity-name> <buzz args...> — one CLI call under another local
# key, in a subshell so the caller's identity is never replaced in this process.
# Every call site passes a literal verb, and there are exactly three:
# `channels members` and `channels list` (reads, used to find an owner and to
# probe relay membership for the roster) and `channels add-member --role member`
# — the only verb here that changes anything, and the only one that is printed
# before it runs.
AS_OUT=""
AS_ERR=""
_as_identity() {
  local ident="$1"; shift
  local f err rc
  f=$(identity_file "$ident")
  err=$(mktemp -t buzz-as) || return 127
  AS_OUT=$( { load_identity "$f" 2>/dev/null || exit 127; "$BUZZ" "$@"; } 2>"$err" )
  rc=$?
  AS_ERR=$(cat "$err" 2>/dev/null)
  rm -f "$err"
  return $rc
}

# find_local_channel_owner <channel> — print "<identity-name>\t<pubkey>" for a
# key in $SESSION_DIR that the relay reports as this channel's owner, else fail.
#
# It has to be done in this order. A non-member cannot see a private channel at
# all: `channels get` returns null and `channels members` returns [] with exit
# 0, so the blocked session cannot read the member list and look the owner up.
# The only identity that can read it is one that is already in the channel, so
# each local key is asked in turn and the one the relay calls "owner" wins.
find_local_channel_owner() {
  local chan="$1" f name pk
  [ -d "$SESSION_DIR" ] || return 1
  for f in "$SESSION_DIR"/*.env; do
    [ -f "$f" ] || continue
    name=$(basename "$f" .env)
    pk=$(identity_field "$f" BUZZ_PUBKEY) || continue
    _as_identity "$name" channels members --channel "$chan" || continue
    printf '%s' "$AS_OUT" | ME="$pk" python3 -c '
import json, os, sys
me = os.environ["ME"]
try:
    rows = json.load(sys.stdin)
except Exception:
    sys.exit(1)
for row in rows if isinstance(rows, list) else []:
    if isinstance(row, dict) and row.get("pubkey") == me and row.get("role") == "owner":
        sys.exit(0)
sys.exit(1)
' || continue
    printf '%s\t%s' "$name" "$pk"
    return 0
  done
  return 1
}

# join_channel <channel-uuid> <channel-name> <my-pubkey>
# Returns 0 only if this session is a member of the channel afterwards.
join_channel() {
  local chan="$1" cname="$2" me="$3" mode owner ident opk tab=$'\t'
  # Self-service first: it is what works on an open channel, and it is not a
  # privileged action, so it needs no announcement.
  if buzz_run channels add-member --channel "$chan" --pubkey "$me" --role member; then
    echo "channel  : joined '$cname' as a member"
    return 0
  fi
  mode=$(setting BUZZ_AUTO_ADMIT 1)
  case "$mode" in
    0|no|off|false)
      note "auto-admit: off (BUZZ_AUTO_ADMIT=$mode) — not looking for an owner key"
      return 1 ;;
  esac
  echo "auto-admit: not a member of '$cname'; checking whether a key in"
  echo "            $SESSION_DIR owns it"
  owner=$(find_local_channel_owner "$chan") || {
    echo "auto-admit: no local key owns '$cname'"
    return 1
  }
  ident=${owner%%"$tab"*}
  opk=${owner#*"$tab"}
  cat <<EOF
auto-admit: '$ident' owns this channel and its key is on this machine.
            owner key : $opk
            from      : $(identity_file "$ident")
            running   : buzz channels add-member --channel $chan \\
                          --pubkey $me --role member
EOF
  if _as_identity "$ident" channels add-member \
       --channel "$chan" --pubkey "$me" --role member; then
    cat <<EOF
auto-admit: granted. This session now has role member in '$cname', authorised by
            the local identity '$ident'. Nothing else was touched: no relay
            membership, no other channel, no role above member, no new key.
            Set BUZZ_AUTO_ADMIT=0 in $CONFIG_FILE to turn this off.
EOF
    return 0
  fi
  note "auto-admit: '$ident' owns '$cname' but add-member failed: ${AS_ERR:-(no detail)}"
  return 1
}

# --- self-diagnosis ----------------------------------------------------------
# The three failures that cost real time are indistinguishable from "the agent
# is ignoring me" unless they are named. Never let a bare 403 through.
diagnose_relay() {   # $1 exit code, $2 stderr, $3 pubkey, $4 relay
  case "$2" in
    *relay_membership_required*)
      cat >&2 <<EOF

  BLOCKED: this session is not a member of the relay yet.
    pubkey : $3
    relay  : $4

  ASK THE USER FOR EXACTLY THIS, IN THESE WORDS, AND NOTHING ELSE:

    "In Buzz Desktop: Invite to community -> Copy link. Paste it here."

  When they paste it, run:
    buzz-connect.sh --invite "<what they pasted>"

  Do not offer alternatives, do not present options, and do not reason about
  other routes. In particular do not propose 'buzz-admin add-member': it writes
  to the relay's Postgres directly, so it does nothing from a machine that is
  not the relay host, and it is the operator's call regardless. One ask, one
  paste, then this is solved for every session on this machine forever.
EOF
      return 0 ;;
  esac
  case "$1" in
    3) note ""
       note "  BLOCKED: the relay rejected this identity's signature (exit 3)."
       note "    ${2:-(no detail)}"
       note "    Fix: delete $(identity_file "${SESSION_NAME:-<name>}") and re-run"
       note "         buzz-connect.sh to mint a fresh identity." ;;
    2) note ""
       note "  BLOCKED: the relay at $4 did not answer usefully (exit 2)."
       note "    ${2:-(no detail)}"
       note "    'no community is configured for this host' means the URL's host:port"
       note "    does not match the relay's configured community — fix BUZZ_RELAY_URL."
       note "    'Connection refused' means nothing is listening: start a relay, or set"
       note "    BUZZ_RELAY_URL in $CONFIG_FILE." ;;
    *) note ""
       note "  BLOCKED: buzz exited $1."
       note "    ${2:-(no error output)}" ;;
  esac
}

diagnose_channel() { # $1 channel uuid, $2 channel name, $3 pubkey, $4 owner-or-empty
  cat >&2 <<EOF

  BLOCKED: relay membership is not channel membership — they are separate gates.
  This session can reach the relay but is not a member of the channel, so
  'messages get' will return [] forever with no error.
    channel : $2 ($1)
    pubkey  : $3

  Ask the channel owner${4:+ ($4)} to run:
    buzz channels add-member --channel $1 --pubkey $3 --role member
EOF
}

# --- watcher liveness --------------------------------------------------------
# Keyed on the session id, not the name, so a /rename mid-watch does not orphan
# the marker and make an armed watcher look unarmed.
watch_marker() {
  printf '%s/.watch-%s' "$SESSION_DIR" "${CLAUDE_CODE_SESSION_ID:-$1}"
}

watcher_pid() {  # prints the pid of a live watcher for this session, else fails
  local m pid
  m=$(watch_marker "$1")
  [ -f "$m" ] || return 1
  pid=$(head -n 1 "$m" 2>/dev/null)
  case "$pid" in ''|*[!0-9]*) rm -f "$m"; return 1 ;; esac
  kill -0 "$pid" 2>/dev/null || { rm -f "$m"; return 1; }   # stale: process gone
  printf '%s' "$pid"
}

# The roster looks at OTHER identities, so it cannot use CLAUDE_CODE_SESSION_ID.
# It goes the long way round: the identity's .meta records the session id it was
# minted under, and the marker is named after that. An identity with no .meta was
# never adopted by a Claude Code session at all, which is worth saying out loud.
identity_watch_state() {   # prints "live <pid> <channel>" | "stale <pid>" | "none" | "unbound"
  local name="$1" sid m pid ch
  sid=$(meta_get "$name" BUZZ_SESSION_ID) || { printf 'unbound'; return 0; }
  m="$SESSION_DIR/.watch-$sid"
  [ -f "$m" ] || { printf 'none'; return 0; }
  pid=$(sed -n '1p' "$m" 2>/dev/null)
  ch=$(sed -n '2p' "$m" 2>/dev/null)
  case "$pid" in ''|*[!0-9]*) printf 'none'; return 0 ;; esac
  if kill -0 "$pid" 2>/dev/null; then printf 'live %s %s' "$pid" "$ch"
  else printf 'stale %s' "$pid"; fi
}

# identity_relay_state NAME — one authenticated read under that identity's key.
# The same probe buzz-connect.sh uses for itself, which is why it is trustworthy:
# "member" here means exactly what "relay : member" means on connect.
identity_relay_state() {
  local name="$1"
  if _as_identity "$name" channels list --limit 1; then printf 'member'; return 0; fi
  case "$AS_ERR" in
    *relay_membership_required*) printf 'not-a-member' ;;
    *) printf 'unknown' ;;
  esac
}

# archived_pubkeys — the relay's current NIP-IA archive snapshot (kind 13535),
# one pubkey per line. `agents archived` verifies the snapshot's authorship and
# signature itself and fails rather than returning a false empty.
archived_pubkeys() {
  buzz_run agents archived || return 1
  printf '%s' "$BUZZ_OUT" | python3 -c '
import json, sys
try:
    doc = json.load(sys.stdin)
except Exception:
    sys.exit(0)
rows = doc.get("archived", []) if isinstance(doc, dict) else doc
for row in rows if isinstance(rows, list) else []:
    if isinstance(row, str):
        print(row)
    elif isinstance(row, dict):
        pk = row.get("pubkey") or row.get("target") or row.get("target_pubkey") or ""
        if pk:
            print(pk)
'
}

# roster_report — every identity on this machine, whether the relay still counts
# it as a member, and whether anything is listening on its behalf.
#
# It never deletes anything. The point is that relay membership has no expiry, so
# identities accumulate silently; the fix is to make them visible, not to guess
# which ones the user is finished with.
roster_report() {
  local f name pk pid ch archived state relay room live=0 unbound=0 total=0 orphan=0
  archived=$(archived_pubkeys 2>/dev/null || printf '')
  echo ""
  echo "roster   : identities in $SESSION_DIR"
  echo "           (one relay call per identity, so this is only done on request)"
  echo ""
  printf '  %-30s %-18s %-16s %-14s %s\n' IDENTITY PUBKEY RELAY WATCHER ROOM
  for f in "$SESSION_DIR"/*.env; do
    [ -f "$f" ] || continue
    total=$((total + 1))
    name=$(basename "$f" .env)
    pk=$(identity_field "$f" BUZZ_PUBKEY || printf '?')
    relay=$(identity_relay_state "$name")
    case "$archived" in *"$pk"*) relay="$relay/archived" ;; esac
    state=$(identity_watch_state "$name")
    room=$(meta_get "$name" BUZZ_SESSION_CHANNEL_NAME || printf '')
    case "$state" in
      "live "*)
        live=$((live + 1))
        pid=${state#live }; ch=${pid#* }; pid=${pid%% *}
        state="live pid $pid"
        # A watcher still polling a room the identity is no longer pinned to is
        # the leak this whole verb exists for: the session went away, the Monitor
        # did not, and it will poll until the Claude Code session ends.
        if [ -z "$room" ]; then
          room="none pinned; still polling ${ch:0:8}"
          orphan=$((orphan + 1))
        fi ;;
      unbound)
        unbound=$((unbound + 1)) ;;
    esac
    printf '  %-30s %-18s %-16s %-14s %s\n' \
      "$name" "${pk:0:16}" "$relay" "$state" "${room:--}"
  done
  [ "$total" != 0 ] || { echo "  (none — run buzz-connect.sh)"; return 0; }
  cat <<EOF

  $total identities, $live with a live watcher.

  WATCHER   live = a Monitor is polling for it now. none = nothing is listening,
            but the identity is still a relay member and still holds a key.
            unbound = no .meta, so no Claude Code session ever adopted it: it was
            minted by hand or is left over from a session that never ran.
  RELAY     member = the relay still accepts writes from that key. Membership has
            no expiry, so it stays a member until an operator removes it.

  Nothing here is pruned automatically — an identity with no watcher is usually a
  session that is between runs, not a dead one. To retire one deliberately, from
  that session, for itself:

    buzz-connect.sh disconnect --retire

  Deleting an identity's .env destroys its keypair: it can never sign again, and
  the name it used in old messages can never be reclaimed. Do that only for an
  identity you can name and know is finished.
EOF
  [ "$orphan" = 0 ] || cat <<EOF

  $orphan watcher(s) are polling a room their identity is no longer pinned to.
  That is a Monitor whose session has moved on; stop it with TaskStop. Until then
  it costs a relay call every 5 seconds and wakes nobody.
EOF
  [ "$unbound" = 0 ] || cat <<EOF

  $unbound identity/identities have no session binding: no .meta, so no Claude
  Code session ever adopted them. They are still relay members and their keys can
  authorise a channel admit (see BUZZ_AUTO_ADMIT), so leaving them in place is a
  decision, not an oversight.
EOF
}

# --- teardown -----------------------------------------------------------------
# Four separable actions, and only the first two are unambiguous, so only the
# first two happen by default:
#
#   1. stop the watcher   — always. It is a Claude Code Monitor, so this script
#                           cannot kill it; it prints the exact TaskStop call.
#   2. say goodbye        — always. A session that stops answering without a DONE
#                           is indistinguishable from one that is just slow.
#   3. leave the channel  — --leave-channel. Right for a finished piece of work,
#                           wrong for a session that reconnects tomorrow.
#   4. retire the identity — --retire. Right for a throwaway worktree, wrong for
#                           anything resumable. Never implicit.
#
# Reads SESSION_NAME, SESSION_DISPLAY, PUBKEY, CHANNEL, CHANNEL_NAME.
teardown() {
  local verb="$1" do_leave="$2" do_retire="$3" note_text="${4:-}"
  local msg pid room
  room="${CHANNEL_NAME:-$CHANNEL}"

  # 1. Goodbye first, while this session is still a channel member — after
  #    `channels leave` the relay refuses the send.
  if [ -n "$CHANNEL" ]; then
    if [ "$verb" = leave ]; then
      msg="DONE $SESSION_DISPLAY: leaving $room"
    else
      msg="DONE $SESSION_DISPLAY: disconnecting, this session is finished"
    fi
    [ -n "$note_text" ] && msg="$msg — $note_text"
    if buzz_run messages send --channel "$CHANNEL" --content "$msg"; then
      echo "goodbye  : posted to $room"
      echo "           $msg"
    else
      note "warning: could not post DONE to $room: ${BUZZ_ERR:-(no detail)}"
      note "         peers will see this session go quiet without an explanation."
    fi
  fi

  # 2. The watcher. A shell script cannot stop a Monitor — print the call, the
  #    mirror of the Monitor(...) that connect prints.
  if pid=$(watcher_pid "$SESSION_NAME"); then
    cat <<EOF
watcher  : running (pid $pid). It is a Claude Code Monitor, so this script
           cannot stop it. Stop it now — otherwise it keeps polling $room
           after this session is gone:

TaskStop(
  task_id: "<the id returned when you armed the Monitor for 'buzz coordination: $room'>"
)

           If that id is lost, 'kill $pid' also ends it — the watcher clears its
           own marker on TERM — but TaskStop is what stops Claude Code tracking
           the task. Confirm with: buzz-connect.sh status
EOF
  else
    echo "watcher  : not running — nothing to stop."
  fi

  # 3. Unpin the room. The pin is what makes a bare `buzz-msg.sh send` post here;
  #    leaving it set after a leave would route messages into a room this session
  #    is no longer in, which fails with 'not a channel member' and reads as a bug.
  meta_unset "$SESSION_NAME" BUZZ_SESSION_CHANNEL
  meta_unset "$SESSION_NAME" BUZZ_SESSION_CHANNEL_NAME
  echo "room     : unpinned. 'buzz-msg.sh send' no longer posts to $room."

  # 4. Opt-in: leave the channel.
  if [ "$do_leave" = 1 ] && [ -n "$CHANNEL" ]; then
    if buzz_run channels leave --channel "$CHANNEL"; then
      cat <<EOF
channel  : left '$room'. The relay evicted this session's live subscriptions
           and posted 'member_left' into the room, so peers see it.
           This is not self-reversible on a private channel: 'channels join' is
           refused with 'restricted: channel is private'. Any remaining member of
           the room can re-add this pubkey, and 'buzz-connect.sh join $room'
           does it with no human involved when the owner's key is in
           $SESSION_DIR.
EOF
    else
      # The session that opened a dedicated room is its owner, so this is the
      # normal outcome of `join <name>` followed by `disconnect --leave-channel`,
      # not an edge case. The relay refuses because a room with no owner can never
      # admit anyone again.
      case "$BUZZ_ERR" in
        *"last owner"*)
          cat >&2 <<EOF
channel  : NOT left. This session owns '$room' and is its last owner, so the
           relay refuses — an ownerless private room can never admit anyone
           again. Choose one, or keep the membership:
             hand it over  buzz channels add-member --channel $CHANNEL \\
                             --pubkey <peer> --role owner
             end the room  buzz channels delete --channel $CHANNEL
EOF
          ;;
        *) note "warning: could not leave '$room': ${BUZZ_ERR:-(no detail)}" ;;
      esac
    fi
  elif [ "$verb" = disconnect ] && [ -n "$CHANNEL" ]; then
    echo "channel  : still a member of '$room' — pass --leave-channel to give up"
    echo "           the membership. Keeping it is what makes a reconnect free."
  fi

  # 5. Opt-in: retire the identity.
  if [ "$do_retire" = 1 ]; then
    retire_identity || return 5
  fi

  # 6. What is left, said plainly. Every teardown leaves residue and pretending
  #    otherwise is how six identities accumulated in the first place.
  cat <<EOF
remains  : keypair  $(identity_file "$SESSION_NAME")  (mode 600)
           relay    this pubkey is still a relay member, and stays one.
           relay_members has no expiry column, so membership lasts until someone
           deletes the row. Nothing this session can run deletes it: the relay
           does accept a self-service leave (NIP-43 kind:28936, which removes the
           sender's own row), but no client builds that event — it is in the relay
           and in buzz-core's kind table and nowhere else, so buzz, buzz-sdk,
           Desktop and the web app cannot send it. The admin remove (kind:9031)
           refuses self-removal outright, and 'buzz-admin remove-member' writes to
           the relay's Postgres directly, so it does nothing unless the operator
           runs it on the relay host. Retiring the identity (--retire) is the
           strongest thing this session can do about itself.
EOF
  return 0
}

# retire_identity — NIP-IA kind:9035 for this session's own pubkey. Self-service
# because the relay's self consent path is actor == target; this never touches
# another identity, and there is no flag here that could make it.
retire_identity() {
  cat <<EOF
retire   : submitting a NIP-IA archive request (kind 9035) for THIS session's own
           identity, $PUBKEY.
           What it does: the relay adds one row to archived_identities and
           republishes its kind:13535 snapshot, so clients and peers can see the
           identity is retired and stop addressing it. Buzz Desktop shows it with
           an 'Archived' flair.
           What it does not do: it does not block this key from reading, writing
           or connecting, does not hide anything it already published, does not
           touch relay membership, and does not remove it from any channel.
           Archival is a signal to readers, not a lock.
           Reversal: 'buzz agents unarchive $PUBKEY' restores the relay's state
           exactly — the archive is that one row and unarchiving deletes it. It
           does not restore the record: the 9035 and 9036 requests are stored,
           publicly readable events, and the archive row's reason and timestamp
           are destroyed rather than rolled back. Re-archiving later keeps the
           first reason and publishes nothing new. So it is reversible, and it is
           not private and not free.
EOF
  if buzz_run agents archive "$PUBKEY" --reason retired; then
    echo "retire   : archived. Peers reading the snapshot will treat '$SESSION_NAME' as retired."
    return 0
  fi
  note "retire   : FAILED — ${BUZZ_ERR:-(no detail)}"
  note "           The identity is unchanged and is still an active relay member."
  return 1
}
