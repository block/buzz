#!/usr/bin/env bash
# buzz-session.sh — the Buzz identity of one Claude Code session.
#
#   buzz-session.sh ensure  [name]   mint or adopt this session's identity
#   buzz-session.sh resolve [name]   "<name>\t<display>\t<pubkey>\t<file>"
#   buzz-session.sh pubkey  [name]   the public key, safe to paste anywhere
#   buzz-session.sh profile [name]   publish the display name to the relay
#   buzz-session.sh list             every known session identity
#
# You normally do not run this. buzz-connect.sh calls it, and buzz-connect.sh is
# the skill's only entry point.
#
# The identity belongs to the SESSION, not to a directory. With no explicit
# name, the name comes from buzz-session-name.sh — the title set with /rename,
# falling back to the worktree directory and then the session id. The env file
# records CLAUDE_CODE_SESSION_ID, so when /rename changes the name the existing
# identity is renamed with it rather than a second keypair being minted.
#
# Identities live in ~/.buzz/sessions/<name>.env (keys, sourced) plus
# <name>.meta (session id and display name, never sourced), both mode 600. The
# secret key is never printed — only the public key, which is all a relay owner
# needs.
set -uo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

NAME_SCRIPT="$HERE/buzz-session-name.sh"
TAB=$'\t'

usage() { die "usage: $0 {ensure|resolve|pubkey|profile|list} [name] [--force]"; }

FORCE=0
cmd=${1:-ensure}
[ $# -gt 0 ] && shift
ARG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=1 ;;
    -*) usage ;;
    *) [ -n "$ARG" ] && usage; ARG="$1" ;;
  esac
  shift
done

# --- name resolution ---------------------------------------------------------
SESSION_NAME=""
SESSION_DISPLAY=""

resolve_names() {
  if [ -n "$ARG" ]; then
    # An explicit name still gets sanitised: it lands in a filename.
    local pair
    pair=$(printf '%s' "$ARG" | "$NAME_SCRIPT" --sanitize --both 2>/dev/null)
    SESSION_NAME=${pair%%"$TAB"*}
    SESSION_DISPLAY=${pair#*"$TAB"}
    [ -n "$SESSION_NAME" ] || die "name '$ARG' sanitises to nothing — choose another"
    return 0
  fi
  local pair
  pair=$("$NAME_SCRIPT" --both)
  SESSION_NAME=${pair%%"$TAB"*}
  SESSION_DISPLAY=${pair#*"$TAB"}
  [ -n "$SESSION_NAME" ] || die "could not resolve a session name (this should be impossible)"
}

# Follow a /rename: if this session already has an identity filed under a
# different name, move it rather than minting a second keypair.
adopt_for_session() {
  [ -n "$ARG" ] && return 0          # explicit name: no adoption, no surprises
  local old
  old=$(identity_for_session) || return 0
  [ "$old" = "$SESSION_NAME" ] && return 0
  if [ -e "$(identity_file "$SESSION_NAME")" ]; then
    note "note: '$SESSION_NAME' is already taken by another identity; keeping '$old'"
    SESSION_NAME="$old"
    return 0
  fi
  mv "$(identity_file "$old")" "$(identity_file "$SESSION_NAME")" \
    || die "could not rename identity '$old' -> '$SESSION_NAME'"
  [ -f "$(meta_file "$old")" ] && mv "$(meta_file "$old")" "$(meta_file "$SESSION_NAME")"
  note "session renamed: $old -> $SESSION_NAME"
}

mint() {
  local file="$1" admin
  admin=$(resolve_bin buzz-admin BUZZ_ADMIN_BIN) || die \
"buzz-admin not found — it mints the keypair.
  Fix: cargo build --release -p buzz-admin
  Or:  export BUZZ_ADMIN_BIN=/path/to/buzz-admin"
  # The relay URL is written into a file that gets sourced, so it is checked
  # rather than trusted, however it reached us.
  case "$RELAY" in
    *[!A-Za-z0-9:/._~%+-]*|'') die "relay URL '$RELAY' contains characters that are not allowed" ;;
  esac
  mkdir -p "$SESSION_DIR"
  umask 077
  # generate-key prints 'Public key: <hex>' / 'Secret key: <hex>'. Pipe it
  # straight into the file writer so the secret never reaches a terminal, a
  # shell variable, or the agent's transcript.
  "$admin" generate-key | FILE="$file" RELAY="$RELAY" python3 -c '
import os, re, sys
pub = sec = None
for line in sys.stdin:
    m = re.search(r"([0-9a-f]{64})", line)
    if not m:
        continue
    if "Public" in line:
        pub = m.group(1)
    elif "Secret" in line:
        sec = m.group(1)
if not (pub and sec):
    sys.exit("could not parse keypair from buzz-admin generate-key")
with open(os.environ["FILE"], "w") as fh:
    fh.write(
        "# Buzz session identity. Sourced, so it holds only hex keys and a URL;\n"
        "# the display name lives in the .meta file beside it.\n"
        "BUZZ_PRIVATE_KEY=%s\n"
        "BUZZ_PUBKEY=%s\n"
        "BUZZ_RELAY_URL=%s\n" % (sec, pub, os.environ["RELAY"])
    )
' || die "keypair generation failed"
  chmod 600 "$file"
}

ensure_identity() {
  resolve_names
  adopt_for_session
  RELAY=$(setting BUZZ_RELAY_URL "http://localhost:3000")
  IDFILE=$(identity_file "$SESSION_NAME")
  if [ ! -f "$IDFILE" ]; then
    mint "$IDFILE"
    IDENTITY_CREATED=1
  else
    IDENTITY_CREATED=0
  fi
  # Bind the identity to this session and keep the display name current, so a
  # later /rename moves this identity instead of minting another one.
  #
  # Only for a resolved name. An explicit name already opts out of adoption, and
  # binding it anyway would mean a /rename in the terminal that happened to run
  # `ensure <name>` renames that identity too — which is wrong for a named agent
  # that outlives the session, and is how a daemon loses its key.
  [ -z "$ARG" ] && [ -n "${CLAUDE_CODE_SESSION_ID:-}" ] \
    && [ "$(meta_get "$SESSION_NAME" BUZZ_SESSION_ID)" != "${CLAUDE_CODE_SESSION_ID}" ] \
    && meta_set "$SESSION_NAME" BUZZ_SESSION_ID "$CLAUDE_CODE_SESSION_ID"
  [ "$(meta_get "$SESSION_NAME" BUZZ_SESSION_DISPLAY_NAME)" = "$SESSION_DISPLAY" ] \
    || meta_set "$SESSION_NAME" BUZZ_SESSION_DISPLAY_NAME "$SESSION_DISPLAY"

  PUBKEY=$(identity_field "$IDFILE" BUZZ_PUBKEY) \
    || die "identity file $IDFILE has no BUZZ_PUBKEY — delete it and re-run"
  RELAY=$(identity_field "$IDFILE" BUZZ_RELAY_URL || printf '%s' "$RELAY")
  RELAY=$(setting BUZZ_RELAY_URL "$RELAY")
}

# --- subcommands -------------------------------------------------------------
case "$cmd" in
  list)
    found=0
    for f in "$SESSION_DIR"/*.env; do
      [ -f "$f" ] || continue
      found=1
      live=""
      watcher_pid "$(basename "$f" .env)" >/dev/null 2>&1 && live=" [watching]"
      printf '%-28s %s  %s%s\n' \
        "$(basename "$f" .env)" \
        "$(identity_field "$f" BUZZ_PUBKEY || echo '?')" \
        "$(meta_get "$(basename "$f" .env)" BUZZ_SESSION_DISPLAY_NAME || echo '-')" \
        "$live"
    done
    [ "$found" = 1 ] || echo "(no session identities yet — run buzz-connect.sh)"
    ;;

  resolve)
    ensure_identity
    printf '%s\t%s\t%s\t%s\n' "$SESSION_NAME" "$SESSION_DISPLAY" "$PUBKEY" "$IDFILE"
    ;;

  pubkey)
    ensure_identity
    printf '%s\n' "$PUBKEY"
    ;;

  profile)
    ensure_identity
    require_buzz
    published=$(meta_get "$SESSION_NAME" BUZZ_PROFILE_NAME || printf '')
    if [ "$published" = "$SESSION_DISPLAY" ] && [ "$FORCE" = 0 ]; then
      echo "profile already published as '$SESSION_DISPLAY'"
      exit 0
    fi
    load_identity "$IDFILE" || die "could not load $IDFILE"
    if buzz_run users set-profile --name "$SESSION_DISPLAY"; then
      meta_set "$SESSION_NAME" BUZZ_PROFILE_NAME "$SESSION_DISPLAY"
      echo "profile published: $SESSION_DISPLAY"
    else
      rc=$?
      note "could not publish profile as '$SESSION_DISPLAY'"
      diagnose_relay "$rc" "$BUZZ_ERR" "$PUBKEY" "$RELAY"
      exit "$rc"
    fi
    ;;

  ensure|new)
    ensure_identity
    [ "$IDENTITY_CREATED" = 1 ] && echo "created identity '$SESSION_NAME'" \
                                || echo "reusing identity '$SESSION_NAME'"
    cat <<EOF
  session  : $SESSION_DISPLAY
  identity : $SESSION_NAME
  pubkey   : $PUBKEY
  relay    : $RELAY
  env file : $IDFILE  (mode 600 — never print its contents)
  metadata : $(meta_file "$SESSION_NAME")
EOF
    ;;

  *) usage ;;
esac
