#!/usr/bin/env bash
# buzz-stream.sh <session-name|-> <channel-uuid> [poll-seconds]
#
# The RECEIVER. Holds the relay connection, filters, and appends one line per new
# peer message to ~/.buzz/stream/<identity>.<channel>.log. It is a daemon: it is
# started with nohup by buzz-connect.sh and by buzz-watch.sh, it runs outside any
# Monitor task, and it is meant to outlive both.
#
# Nothing reads its stdout. Waking a session is buzz-watch.sh's job — it tails
# this log under Monitor. The split is the guardrail: Monitor-hosted watchers
# have been seen dying (exit 144) where the same command under nohup did not, so
# the part that must never miss a message is kept out of Monitor entirely. A
# Monitor death now costs the wake, not the messages: they keep arriving here and
# re-arming replays them from the stored offset.
#
# --- how a message gets into the log ------------------------------------------
# Two sources, never running at the same time, feeding one dedupe:
#
#   PUSH   `buzz messages subscribe` holds a NIP-42-authenticated WebSocket open
#          and prints one JSON object per line the instant the relay pushes an
#          event. Delivery is a socket write, not an interval.
#   SWEEP  `buzz messages get --since` over HTTP, once before every stream and
#          again every time one ends. The safety net — and the ONLY source on a
#          build of buzz with no `subscribe` verb, in which case this is exactly
#          the polling loop this skill has always used.
#
# The sweep is not redundant. A subscription the relay has quietly stopped
# matching against is silent and still heartbeating, exactly like a quiet
# channel. `--reconnect-after` ends a healthy stream on a schedule so the sweep
# gets a turn to find out which one it was, and `--since` covers the gap while
# the socket was down.
#
# Seven details this encodes; do not "simplify" them away:
#   1. `buzz messages get --since <ts>` is INCLUSIVE. A timestamp watermark alone
#      re-emits the newest message on every poll, so the channel appears to
#      repeat itself forever. Dedupe on event id; --since only bounds the query.
#      That dedupe is also what lets push and HTTP feed one filter safely.
#   2. The seen-set is primed from history at startup, so starting a receiver
#      does not append the whole backlog. A prime that FAILED is not a prime that
#      found an empty channel — see below.
#   3. A session must never react to its own messages — filter on own pubkey.
#      This is also why the log is per identity and not per channel.
#   4. The stream runs as a backgrounded job under `set -m`, waited on rather
#      than run in the foreground. Bash defers a trap until a foreground command
#      returns, so a foreground stream would ignore TERM for as long as it lived
#      and leave an authenticated WebSocket behind. `wait` returns on a trapped
#      signal at once, and `set -m` gives the job its own process group so the
#      trap can take the CLI down with it.
#   5. Never run the stream inside $(...): command substitution captures stdout,
#      and stdout is where the notifications are.
#   6. The heartbeat is what makes this observable. A receiver wedged on a socket
#      is still a running process, so a pid check alone would call it healthy.
#   7. Losing push is written to the log, once, and so is getting it back.
set -uo pipefail
set -m   # each background job in its own process group — see note 4

# Watchers hosted by Monitor have been observed exiting 144 (128+16 = SIGURG on
# Darwin). No mechanism is claimed, and one reading is already ruled out: bash
# ignores SIGURG by default, so a bare SIGURG cannot produce 144 on its own —
# verified on Darwin 25. This is cheap insurance, not a fix. An ignored
# disposition is inherited across fork AND exec, so it covers the CLI, python3
# and every subshell below here. Nothing in this script wants SIGURG.
trap '' URG

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$HERE/lib.sh"

NAME="${1:?usage: buzz-stream.sh <session-name|-> <channel-uuid> [poll-seconds]}"
# addressed (default) | all | mentions — see worth_waking() in the filter.
WAKE="${BUZZ_WAKE:-addressed}"
CH="${2:?usage: buzz-stream.sh <session-name|-> <channel-uuid> [poll-seconds]}"
SLEEP="${3:-5}"

require_buzz

if [ "$NAME" = "-" ]; then
  IDENT=$("$HERE/buzz-session.sh" resolve) || exit 1
  NAME=$(printf '%s' "$IDENT" | cut -f1)
  ENV_FILE=$(printf '%s' "$IDENT" | cut -f4)
else
  ENV_FILE=$(identity_file "$NAME")
fi
[ -f "$ENV_FILE" ] || die \
"no identity '$NAME' in $SESSION_DIR — run $HERE/buzz-connect.sh first"

load_identity "$ENV_FILE" || die "could not load $ENV_FILE"
export RUST_LOG="${RUST_LOG:-error}"   # keep tracing out of the log

mkdir -p "$STREAM_DIR"
chmod 700 "$STREAM_DIR" 2>/dev/null || true
LOG=$(stream_log "$NAME" "$CH")
ERR=$(stream_err "$NAME" "$CH")
HB=$(stream_hb "$NAME" "$CH")
PIDF=$(stream_pidf "$NAME" "$CH")
touch "$LOG"

# Everything this process says goes to a file that outlives it. Monitor reaps its
# own task output, which is why three watcher deaths produced no diagnosis; this
# receiver is not under Monitor and its stderr is kept deliberately.
exec 2>>"$ERR"
printf '%s start pid=%s identity=%s channel=%s\n' \
  "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$$" "$NAME" "$CH" >&2

# --- one receiver per identity per channel ------------------------------------
# Two receivers on one log would double every notification. `mkdir` is the lock
# because it is atomic; a lock whose recorded pid is gone was left by a receiver
# that was killed rather than asked to stop, and is taken over.
LOCK="$(stream_base "$NAME" "$CH").lock"
if ! mkdir "$LOCK" 2>/dev/null; then
  OTHER=$(cat "$PIDF" 2>/dev/null)
  case "$OTHER" in
    ''|*[!0-9]*) ;;
    *) if kill -0 "$OTHER" 2>/dev/null; then
         printf '%s exit: receiver pid=%s already holds this channel\n' \
           "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$OTHER" >&2
         exit 0
       fi ;;
  esac
  printf '%s taking over a lock left by a dead receiver (pid=%s)\n' \
    "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "${OTHER:-unknown}" >&2
fi
printf '%s\n' "$$" > "$PIDF"

# --- clear out what a killed predecessor left running -------------------------
# Nothing runs in a SIGKILLed process, so its stream job — an authenticated
# WebSocket and a filter, in their own process group — is orphaned and keeps
# appending to this very log. That is not merely a leak: alongside a fresh
# receiver it duplicates every message, because the orphan holds its own copy of
# a seen-set nothing will ever reconcile. Whoever starts next is the only thing
# that can clean it up, so this is where it happens.
SUBPGF="$(stream_base "$NAME" "$CH").subpg"
if [ -f "$SUBPGF" ]; then
  OLDPG=$(cat "$SUBPGF" 2>/dev/null)
  case "$OLDPG" in
    ''|*[!0-9]*) ;;
    *) if kill -0 -- "-$OLDPG" 2>/dev/null; then
         printf '%s killing a stream job orphaned by a previous receiver (pgid=%s)\n' \
           "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$OLDPG" >&2
         kill -TERM -- "-$OLDPG" 2>/dev/null
       fi ;;
  esac
  rm -f "$SUBPGF"
fi

SEEN=$(mktemp -t buzz-stream-seen)
SUBERR=$(mktemp -t buzz-stream-err)
STREAM_JOB=""
HB_JOB=""
cleanup() {
  local rc=$?
  trap - EXIT INT TERM
  [ -n "$STREAM_JOB" ] && kill -TERM -- "-$STREAM_JOB" 2>/dev/null
  [ -n "$HB_JOB" ] && kill -TERM "$HB_JOB" 2>/dev/null
  printf '%s exit status=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$rc" >> "$ERR"
  rm -f "$SEEN" "$SUBERR" "$PIDF" "$HB" "$SUBPGF"
  rmdir "$LOCK" 2>/dev/null
  exit 0
}
trap cleanup EXIT INT TERM

# --- heartbeat ----------------------------------------------------------------
# The only thing that distinguishes a working receiver from a wedged one. It has
# to tick faster than the loop, because a healthy stream blocks for minutes and
# a per-iteration heartbeat would look stale for most of that time.
#
# The loop condition is load-bearing. Nothing runs in a SIGKILLed process, so a
# receiver killed outright leaves this child behind, and a heartbeat that kept
# ticking for a pid that no longer exists made liveness flap between the orphan's
# writes and its replacement's. It stops within one tick of its parent going.
TICK=$(stream_tick)
PARENT=$$
heartbeat() {
  while kill -0 "$PARENT" 2>/dev/null; do
    printf '%s\n%s\n' "$PARENT" "$(date +%s)" > "$HB"
    sleep "$TICK"
  done
}
heartbeat &
HB_JOB=$!

# --- prime: everything already in the channel counts as backlog ---------------
# A prime that FAILS is not a prime that found an empty channel, and conflating
# them is expensive. An unreachable relay at start leaves the seen-set empty, and
# the first read that does succeed then appends the entire backlog as if it were
# news. So record the failure and let the first successful sweep do the priming.
# Nothing is lost by swallowing that batch: the relay was down, so nothing in it
# was posted after this receiver started.
SWEEP_MODE=array
if PRIME=$("$BUZZ" messages get --channel "$CH" --limit 200 2>/dev/null); then
  printf '%s' "$PRIME" | python3 -c '
import json, sys
try:
    for m in json.load(sys.stdin):
        if m.get("id"):
            print(m["id"])
except Exception:
    pass
' > "$SEEN" 2>/dev/null || true
else
  SWEEP_MODE=prime
  note "could not read history to prime the seen-set — the first successful read"
  note "will be treated as backlog rather than appended as new messages."
fi
unset PRIME

# One filter for every source. MODE=array reads the JSON array `messages get`
# returns; MODE=stream reads the newline-delimited objects `messages subscribe`
# writes; MODE=prime reads an array and records it as seen without emitting.
# Everything after parsing is shared, so a message is logged exactly once
# whichever path carried it.
#
# Lines beginning with '#' are this script reporting on the connection. They are
# not events and never come from the relay.
FILTER=$(cat <<'PY'
import json, os, sys

me = os.environ["ME"]
path = os.environ["SEEN"]
mode = os.environ.get("MODE", "array")
wake = os.environ.get("WAKE", "addressed")
myname = os.environ.get("MYNAME", "").lower()

# Which messages are worth a model turn.
#
# Every wake costs an inference in every listening session, so an ambient
# channel scales badly in peers: three sessions turn one STATUS into two turns
# with nothing to answer. buzz-acp defaults to mention-only for this reason.
#
# Mention-only is too strict here, because the protocol has messages that are
# nobody's to answer and everybody's to know — a CLAIM is how a peer learns not
# to touch a path. So split by audience, which the verbs already encode:
#
#   addressed to me   ASK/ANSWER naming me, or a p-tag mention  -> wake
#   shared state      CLAIM RELEASE BLOCKED                     -> wake
#   informational     HELLO DONE STATUS from someone else       -> log only
#
# Nothing is dropped. Everything still lands in the log, so `buzz-msg.sh read`
# and the next catch-up show the whole conversation. The only judgement here is
# whether it is worth interrupting for.
SHARED = ("CLAIM", "RELEASE", "BLOCKED")
DIRECTED = ("ASK", "ANSWER")


def worth_waking(m, body):
    if wake == "all":
        return True
    tags = m.get("tags") or []
    if any(t and t[0] == "p" and len(t) > 1 and t[1] == me for t in tags):
        return True                       # explicitly mentioned
    if wake == "mentions":
        return False
    head = body[:120].upper()
    if head.startswith(SHARED):
        return True                       # affects paths I may be editing
    if head.startswith(DIRECTED) and myname and myname in body[:200].lower():
        return True                       # named in an ASK or ANSWER
    return False

with open(path) as fh:
    seen = set(fh.read().split())
fresh = []


def remember():
    global fresh
    if fresh:
        with open(path, "a") as fh:
            fh.write("\n".join(fresh) + "\n")
        fresh = []


def emit(m):
    eid = m.get("id")
    if not eid or eid in seen:
        return
    seen.add(eid)
    fresh.append(eid)
    if mode == "prime":              # record as backlog, wake nobody
        return
    if m.get("pubkey") == me:        # never react to our own messages
        return
    if m.get("kind") not in (9, 1):  # chat kinds only
        return
    who = m.get("pubkey", "")[:8]
    body = " ".join(m.get("content", "").split())[:400]
    if not worth_waking(m, body):
        return                       # in the log, not worth an inference
    print("[buzz] %s: %s" % (who, body), flush=True)


if mode in ("array", "prime"):
    try:
        msgs = json.load(sys.stdin)
    except Exception:
        msgs = []
    for m in sorted(msgs, key=lambda x: x.get("created_at", 0)):
        emit(m)
else:
    # readline, not `for line in sys.stdin`: iteration reads ahead, and a line
    # sitting in a read-ahead buffer is a message not yet in the log.
    for line in iter(sys.stdin.readline, ""):
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            print("[buzz] %s" % line[1:].strip(), flush=True)
            continue
        try:
            emit(json.loads(line))
        except Exception:
            continue
        # Persist per event, not at exit: this process is killed, not asked to
        # stop, and a seen-set lost on TERM replays the room on the next start.
        remember()

remember()
PY
)

# run_filter MODE — one short-lived filter appending to the log. Line-buffered
# through the append so a tail sees each notification as it is written.
run_filter() {
  ME="$BUZZ_PUBKEY" SEEN="$SEEN" MODE="$1" WAKE="$WAKE" MYNAME="$NAME" \
    python3 -u -c "$FILTER" >> "$LOG"
}

# say <text> — a connection-state note, through the same filter so the wording
# that reaches a session is decided in exactly one place.
say() { printf '#%s\n' "$1" | run_filter stream; }

# Does this build of buzz have the streaming verb? A prebuilt CLI predating it
# does not, and that must degrade to polling rather than fail: latency is a
# nicety, hearing your peers is not.
PUSH=0
if "$BUZZ" messages subscribe --help >/dev/null 2>&1; then PUSH=1; fi
printf '%s mode=%s poll=%ss\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" \
  "$([ "$PUSH" = 1 ] && echo push+sweep || echo poll-only)" "$SLEEP" >&2

# The sweep's --since watermark. Bounded by BUZZ_WATCH_WINDOW on the first pass
# only; after that it tracks the previous sweep, so however long a stream runs
# between sweeps it cannot open a hole the next query does not cover. The skew
# absorbs clock drift between this machine and the relay.
WINDOW="${BUZZ_WATCH_WINDOW:-300}"
SKEW=30
LAST_SWEEP=$(( $(date +%s) - WINDOW ))

# How long a healthy stream may run before it is torn down so a sweep can happen.
RESUB="${BUZZ_WATCH_RESUBSCRIBE:-300}"
# Silence longer than this, heartbeats included, means the socket is dead. The
# relay heartbeats every 30s, so this must clear several of those.
IDLE="${BUZZ_WATCH_IDLE:-90}"
# A stream that lasted this long definitely connected, authenticated and
# subscribed. It has to sit above the CLI's 20s NIP-42 challenge timeout, or a
# relay that accepts the TCP connection and never sends a challenge would score
# as a healthy connection.
UP_AFTER="${BUZZ_WATCH_UP_AFTER:-25}"

FAILS=0
DOWN=0

# stream_once — hold the WebSocket open until it stops delivering, then leave the
# seconds it lasted in STREAM_RAN. That duration is the only signal for telling
# "the relay is refusing us" from "the connection was fine and the schedule ended
# it": both arrive as a non-zero exit.
STREAM_RAN=0
stream_once() {
  local started resub="$RESUB"
  # While degraded, cut the schedule right down. Recovery is only observable
  # when a stream ENDS having lasted, so on the normal schedule a receiver that
  # got its push path back would record that up to RESUB seconds late — and
  # until then the log's last word is that it is polling, which is false.
  [ "$DOWN" = 1 ] && resub=$(( UP_AFTER + 5 ))
  started=$(date +%s)
  : > "$SUBERR"
  {
    "$BUZZ" messages subscribe --channel "$CH" \
        --since "$(( LAST_SWEEP - SKEW ))" \
        --idle-timeout "$IDLE" --reconnect-after "$resub" 2>"$SUBERR" \
      | run_filter stream
  } &
  STREAM_JOB=$!
  # Recorded so a successor can kill this group if this receiver is killed
  # outright and never gets to clean up after itself.
  printf '%s\n' "$STREAM_JOB" > "$SUBPGF"
  wait "$STREAM_JOB" 2>/dev/null
  STREAM_JOB=""
  rm -f "$SUBPGF"
  cat "$SUBERR" >> "$ERR" 2>/dev/null
  STREAM_RAN=$(( $(date +%s) - started ))
}

while true; do
  # 1. Sweep. Closes whatever gap the last stream left behind, and carries the
  #    whole receiver when there is no push path.
  NOW=$(date +%s)
  OUT=$("$BUZZ" messages get --channel "$CH" --since "$(( LAST_SWEEP - SKEW ))" \
        --limit 200 2>/dev/null) || OUT=""
  if [ -n "$OUT" ]; then
    LAST_SWEEP="$NOW"
    printf '%s' "$OUT" | run_filter "$SWEEP_MODE"
    SWEEP_MODE=array   # only the first read after a failed prime is backlog
  fi

  if [ "$PUSH" != 1 ]; then
    sleep "$SLEEP"
    continue
  fi

  # 2. Push. Blocks here for as long as the relay keeps the subscription alive.
  stream_once

  # A stream that lasted was a working connection, whatever ended it. One that
  # died sooner was refused, or never established.
  if [ "$STREAM_RAN" -ge "$UP_AFTER" ]; then
    FAILS=0
    if [ "$DOWN" = 1 ]; then
      say "relay stream restored — back to push delivery"
      DOWN=0
    fi
    continue
  fi

  FAILS=$(( FAILS + 1 ))
  # Say it once, on the second consecutive failure. A single blip is not worth
  # waking anyone for, and a flapping link must not become a notification storm.
  if [ "$DOWN" = 0 ] && [ "$FAILS" -ge 2 ]; then
    WHY=$(tr -d '\r' < "$SUBERR" | tail -n 1 | cut -c1-160)
    say "relay stream is down, polling every ${SLEEP}s instead — ${WHY:-no detail}"
    DOWN=1
  fi

  # Back off, but never past the poll interval: while the stream is down the
  # sweep at the top of the loop is the only thing delivering, and slowing that
  # below the interval the caller asked for would make this worse than polling.
  BACKOFF=$(( FAILS * FAILS ))
  [ "$BACKOFF" -gt "$SLEEP" ] && BACKOFF="$SLEEP"
  sleep "$BACKOFF"
done
