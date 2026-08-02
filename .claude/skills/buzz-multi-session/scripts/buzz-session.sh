#!/usr/bin/env bash
# buzz-session.sh — give one Claude Code session its own Buzz identity.
#
#   buzz-session.sh new    [name]   mint (or reuse) an identity, print its pubkey
#   buzz-session.sh env    [name]   print the shell snippet that loads it
#   buzz-session.sh pubkey [name]   print just the pubkey
#   buzz-session.sh list            list known session identities
#
# `name` defaults to "<repo>-<worktree-dir>" derived from the current git
# worktree, so three worktrees of one repo get three distinct, attributable
# identities without anyone having to invent names.
#
# Identities live in ~/.buzz/sessions/<name>.env, mode 600. The secret key is
# never printed — only the public key, which is what a relay owner needs.
set -euo pipefail

DIR="${BUZZ_SESSION_DIR:-$HOME/.buzz/sessions}"

die() { printf '%s\n' "$*" >&2; exit 1; }

# Resolve the binaries: PATH first, then a release build in the enclosing
# checkout (the common case for someone hacking on block/buzz).
resolve_bin() {
  local name="$1" var="$2" found
  found="${!var:-}"
  if [ -n "$found" ]; then printf '%s' "$found"; return 0; fi
  if found=$(command -v "$name" 2>/dev/null); then printf '%s' "$found"; return 0; fi
  local root
  if root=$(git rev-parse --show-toplevel 2>/dev/null); then
    for cand in "$root/target/release/$name" "$root/target/debug/$name"; do
      [ -x "$cand" ] && { printf '%s' "$cand"; return 0; }
    done
  fi
  return 1
}

derive_name() {
  git rev-parse --is-inside-work-tree >/dev/null 2>&1 \
    || die "not inside a git worktree — pass an explicit session name"
  local root common repo wt
  root=$(git rev-parse --show-toplevel)
  common=$(cd "$(git rev-parse --git-common-dir)" && pwd)
  repo=$(basename "$(dirname "$common")")
  wt=$(basename "$root")
  printf '%s' "$repo-$wt" | tr -c 'a-zA-Z0-9._-' '-'
}

file_for() { printf '%s/%s.env' "$DIR" "$1"; }

pubkey_of() {
  local f; f=$(file_for "$1")
  [ -f "$f" ] || die "no identity for '$1' — run: $0 new $1"
  grep '^BUZZ_PUBKEY=' "$f" | cut -d= -f2
}

cmd=${1:-new}
[ $# -gt 0 ] && shift || true

case "$cmd" in
  list)
    shopt -s nullglob
    found=0
    for f in "$DIR"/*.env; do
      found=1
      printf '%-32s %s\n' "$(basename "$f" .env)" "$(grep '^BUZZ_PUBKEY=' "$f" | cut -d= -f2)"
    done
    [ "$found" = 1 ] || echo "(no session identities yet)"
    exit 0
    ;;

  env)
    name=${1:-$(derive_name)}
    f=$(file_for "$name")
    [ -f "$f" ] || die "no identity for '$name' — run: $0 new $name"
    echo "set -a; . $f; set +a"
    exit 0
    ;;

  pubkey)
    pubkey_of "${1:-$(derive_name)}"
    exit 0
    ;;

  new)
    name=${1:-$(derive_name)}
    ;;

  *)
    die "usage: $0 {new|env|pubkey|list} [name]"
    ;;
esac

BUZZ=$(resolve_bin buzz BUZZ_BIN) \
  || die "buzz not found on PATH — build it with 'cargo build --release -p buzz-cli' or set BUZZ_BIN"
RELAY="${BUZZ_RELAY_URL:-http://localhost:3000}"
FILE=$(file_for "$name")
mkdir -p "$DIR"

if [ -f "$FILE" ]; then
  echo "reusing existing identity '$name'"
else
  ADMIN=$(resolve_bin buzz-admin BUZZ_ADMIN_BIN) \
    || die "buzz-admin not found — build it with 'cargo build --release -p buzz-admin' or set BUZZ_ADMIN_BIN"
  umask 077
  # generate-key prints "Public key:  <hex>" / "Secret key:  <hex>". Pipe it
  # straight into the file writer so the secret never reaches a terminal, a
  # shell variable, or the agent's transcript.
  "$ADMIN" generate-key \
    | NAME="$name" FILE="$FILE" RELAY="$RELAY" python3 -c '
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
        "# Buzz session identity: %s\n"
        "BUZZ_PRIVATE_KEY=%s\n"
        "BUZZ_PUBKEY=%s\n"
        "BUZZ_RELAY_URL=%s\n" % (os.environ["NAME"], sec, pub, os.environ["RELAY"])
    )
'
  chmod 600 "$FILE"
  echo "created identity '$name'"
fi

PUB=$(pubkey_of "$name")
cat <<EOF

  identity : $name
  pubkey   : $PUB
  relay    : $RELAY
  env file : $FILE  (mode 600, never print its contents)

Load it in this session's shell:
  set -a; . $FILE; set +a

Then (once the relay owner has added this pubkey to the relay AND the channel):
  $BUZZ messages get --channel <channel-uuid> --limit 20
EOF
