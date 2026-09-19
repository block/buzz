#!/usr/bin/env bash
#
# buzz-adopt-prod-agents.sh — copy your installed (production) Buzz agent
# records + owner identity into the dev app-data store so a dev build boots as
# the SAME agents and the SAME owner npub as your installed DMG.
#
# WHY
#   `just reset` deletes the entire dev app-data dir (agent records included),
#   every buzz-desktop-dev keychain entry, and the `_dev_migration_v1` marker.
#   The next dev boot sees an empty store, so key-less records mint fresh
#   keypairs against the prod relay → duplicate agent instances, and a fresh
#   owner key → auth-tag split-brain. This script restores the two things the
#   reset wipes that Buzz cannot re-derive on its own: the agent RECORDS and
#   the OWNER identity nsec. Everything else the app repairs itself on next boot
#   (see WHAT THIS DOES NOT DO).
#
# WHEN
#   Run AFTER `just reset`, BEFORE the first dev boot (`just production` / dev).
#   Any running DEV build must be closed; the installed DMG may stay open (it is
#   only a read-only source here — see preflight 1a). Idempotent: refuses to
#   clobber a non-empty dev store unless --force.
#
# WHAT THIS DOES NOT DO (verified against block/buzz origin/main)
#   - It does NOT touch the buzz-desktop-dev keychain. Agent private keys are
#     copied prod→dev automatically at next dev boot by
#     `migrate_agent_keys_to_dev_service` (desktop/src-tauri/src/managed_agents/
#     storage.rs:460): the reset wiped the `_dev_migration_v1` marker, so it
#     re-runs and copies `agent:<pubkey>` keys for every pubkey in the restored
#     records. It only ever failed to help before because the reset also wiped
#     the RECORDS — step "records" below fixes exactly that.
#   - The owner identity is written as a plaintext `identity.key` file, NOT into
#     the dev keychain. Next dev boot's legacy-import path adopts it into the
#     dev keyring (app_state.rs:583, `ReachableButEmpty` branch =
#     the post-reset empty-dev-keyring state) with the crash-safe
#     marker-before-delete ordering already shipped. The plaintext file exists
#     only until that first boot consumes it.
#   - It does NOT copy baked build-time agent defaults. The prod DMG bakes a
#     provider and default env vars at compile time via `option_env!` in
#     `build.rs` (driven by `BUZZ_BUILD_BUZZ_AGENT_PROVIDER` and
#     `BUZZ_BUILD_AGENT_ENV`). These are compiled into the binary, not stored in
#     app-data — there is no file here to copy. Dev parity means setting those
#     env vars when invoking the dev build:
#       export BUZZ_BUILD_BUZZ_AGENT_PROVIDER="databricks_v2"
#       export BUZZ_BUILD_AGENT_ENV=$'DATABRICKS_HOST=...\nDATABRICKS_MODEL=...'
#       just production
#     Do NOT add these to .env — `set dotenv-load := true` in the Justfile
#     would bake them into every recipe in that checkout, including test runs.
#
# FLAGS
#   --dry-run   Print every action; write nothing.
#   --force     Proceed even if the dev store is non-empty (overwrites records
#               and identity.key). Never bypasses the running-dev-build check.
#   -h|--help   This header.
#
# PATH SAFETY
#   The prod store is strictly read-only — nothing under prod is ever created,
#   read-modified, or deleted. Prod and dev are hardcoded canonical constants,
#   so there is no override path to canonicalize; the only writes go to the
#   fixed dev dir. As a guard against odd on-disk state, the script refuses to
#   run if the dev store dir/agents subdir is a symlink, or if the prod root or
#   any prod source (agents/, the record files, agents/teams/) is a symlink, or
#   if prod and an existing dev resolve to the same or overlapping tree.
#
# ENV OVERRIDES (read-only against prod; exist so the identity read can be
# fixture-tested against a scratch keychain without touching real secrets)
#   BUZZ_KEYCHAIN_SVC   keychain service    (default buzz-desktop)
#   BUZZ_KEYCHAIN_ACCT  keychain account    (default secrets)
#
set -euo pipefail

# --- config -----------------------------------------------------------------
# Prod/dev app-data dirs are hardcoded canonical constants — the only paths a
# real invocation ever uses. Keychain service/account stay overridable because
# they are read-only against prod (used to fixture-test the identity read).
SUPPORT="$HOME/Library/Application Support"
PROD_DIR="$SUPPORT/xyz.block.buzz.app"
DEV_DIR="$SUPPORT/xyz.block.buzz.app.dev"
KEYCHAIN_SVC="${BUZZ_KEYCHAIN_SVC:-buzz-desktop}"
KEYCHAIN_ACCT="${BUZZ_KEYCHAIN_ACCT:-secrets}"

DRY_RUN=0
FORCE=0

RECORD_FILES=(managed-agents.json personas.json teams.json global-agent-config.json)

# --- helpers ----------------------------------------------------------------
say()  { printf '%s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
warn() { printf 'WARN: %s\n' "$*" >&2; }
die()  { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0; }

have() { command -v "$1" >/dev/null 2>&1; }

# Temp paths (files/dirs) to remove on exit. A cleanup FUNCTION consumes this
# array so no path is ever interpolated into trap source — a path containing
# an apostrophe or space cannot break the trap or leave a temp behind.
STAGE_TEMPS=()
# shellcheck disable=SC2329  # invoked indirectly via `trap cleanup_stages EXIT`
cleanup_stages() {
  local p
  for p in "${STAGE_TEMPS[@]:+${STAGE_TEMPS[@]}}"; do
    [[ -e "$p" ]] && rm -rf "$p"
  done
  return 0   # never let the trap's last test override the script's exit status
}
trap cleanup_stages EXIT

# Create a mode-0600 staging file next to its final destination so the later
# rename is same-filesystem atomic. mktemp creates 0600 by default; the umask
# guards against a lenient inherited default. Echoes the temp path.
new_stage() { ( umask 077; mktemp "$1.stage.XXXXXX" ); }

# Atomically move a validated 0600 stage file over its destination. `mv -f`
# replaces the name (never writes through a symlink at $2) so an interrupted
# run can never leave a partial file at the live path.
finalize() { chmod 600 "$1"; mv -f "$1" "$2"; }

# Atomically exchange two existing directories via renamex_np(RENAME_SWAP) — the
# whole-bundle commit primitive. bash cannot call the syscall, so python3 drives
# it. Fail closed: any unavailability or nonzero syscall result exits nonzero so
# the caller aborts with the live store untouched (never a sequential fallback).
swap_dirs() {
  python3 - "$1" "$2" <<'PY'
import ctypes, os, sys
a, b = sys.argv[1], sys.argv[2]
try:
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    renamex_np = libc.renamex_np
except (OSError, AttributeError) as e:
    sys.stderr.write("renamex_np unavailable: %s\n" % e); sys.exit(1)
renamex_np.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
RENAME_SWAP = 0x2
if renamex_np(os.fsencode(a), os.fsencode(b), RENAME_SWAP) != 0:
    e = ctypes.get_errno()
    sys.stderr.write("renamex_np(RENAME_SWAP) failed: errno %d (%s)\n" % (e, os.strerror(e)))
    sys.exit(1)
PY
}

# Strip the volatile runtime_pid field from a managed-agents store (JSON array
# of ManagedAgentRecord; field verified at types.rs:317). Reads $1, writes $2.
# Nonzero exit on malformed input so callers can abort before the rename.
strip_runtime_pid() {
  local src="$1" dst="$2"
  if have jq; then
    jq 'if type=="array" then map(del(.runtime_pid)) else del(.runtime_pid) end' "$src" > "$dst"
  elif have python3; then
    python3 - "$src" "$dst" <<'PY'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
with open(src) as f:
    data = json.load(f)
def strip(r):
    if isinstance(r, dict):
        r.pop("runtime_pid", None)
    return r
data = [strip(r) for r in data] if isinstance(data, list) else strip(data)
with open(dst, "w") as f:
    json.dump(data, f, indent=2)
PY
  else
    die "need jq or python3 to strip runtime_pid"
  fi
}

# --- arg parse --------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --force)   FORCE=1 ;;
    -h|--help) usage ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
  shift
done

[[ $DRY_RUN -eq 1 ]] && say "== DRY RUN — no changes will be written =="

# --- step 1: preflight ------------------------------------------------------
say "[1/4] Preflight"

# 1a. Refuse if a running DEV build is detected; a running installed DMG is
#     allowed (read-only detection; never kills). The main app binary is
#     `buzz-desktop` for both the installed DMG
#     (/Applications/Buzz.app/Contents/MacOS/buzz-desktop) and dev builds
#     (target/<profile>/buzz-desktop via `tauri dev`). Match that path component
#     exactly so sidecars/helpers (buzz, buzz-dev-mcp, buzz-agent) don't
#     false-positive.
#
#     WHY dev blocks but the DMG doesn't: step 2 atomically swaps the entire dev
#     agents/ dir and step 3 stages identity.key for the next dev boot to
#     consume — doing that under a live dev process is exactly the corruption
#     the check must prevent. The DMG is only a read-only source: the app writes
#     managed-agents.json via tmp+atomic-rename (managed_agents/storage.rs:603
#     on origin/main), so a running DMG can never hand us a torn file. The only
#     residual is cross-file skew if agents/teams are mutated in the DMG during
#     the seconds this runs — a warn, self-healing on re-run, not a block.
#
#     Classification per PID resolves the true post-exec executable path via
#     `lsof -d txt` (the running vnode, symlinks already resolved by the kernel;
#     argv[0]/ps comm can lie). Fail closed: the DMG path is the ONLY allow;
#     anything else that is still alive blocks. A PID that exits between pgrep
#     and resolution (path unresolvable AND process gone) is ignored — it is no
#     longer running. A PID still alive but unresolvable (permissions, exotic
#     state) blocks.
ALLOWED_PROD_EXE="/Applications/Buzz.app/Contents/MacOS/buzz-desktop"

# Echo a PID's true executable path (first txt-mapped vnode), or empty if none.
# lsof exits nonzero when the PID is gone; callers use `|| true` so a raced exit
# under `set -e` yields an empty path (handled as raced-exit) instead of aborting.
pid_exe_path() { lsof -a -p "$1" -d txt -Fn 2>/dev/null | awk '/^n/{print substr($0,2); exit}'; }

# True only for the documented clean no-match — the PID has genuinely exited.
# `ps -p` fails BOTH for a gone PID and for other errors, and `kill -0` fails
# both for ESRCH and EPERM (a live process the caller can't signal), so neither
# status alone distinguishes gone from alive-but-indeterminate. Classify by all
# three signals and fail closed: gone == the ONE documented clean no-match shape
# (macOS `ps` exits EXACTLY 1 with empty stdout and empty stderr). A live PID
# (incl. one we lack permission to signal) exits 0 and prints its pid; any other
# status, any stderr diagnostic, or a setup failure (mktemp/rm) is indeterminate
# and returns non-gone → blocking. `pid_gone` runs as an `elif` condition where
# `errexit` is suppressed, so it must classify failures explicitly rather than
# rely on `set -e` to abort — hence the explicit `|| return 1` on setup steps.
pid_gone() {
  local out err rc
  err="$(mktemp)" || return 1
  if out="$(ps -p "$1" -o pid= 2>"$err")"; then rc=0; else rc=$?; fi
  local had_err=0; [[ -s "$err" ]] && had_err=1
  rm -f "$err" || return 1
  [[ $rc -eq 1 && -z "$out" && $had_err -eq 0 ]]
}

running_pids="$(pgrep -f '/buzz-desktop( |$)' 2>/dev/null || true)"
dmg_running=0
dev_blocking=()   # "pid:reason" for each PID that blocks the run
if [[ -n "$running_pids" ]]; then
  # Canonicalize the allow-path once (resolve any symlink in the DMG bundle
  # path) so a symlinked DMG binary still matches the resolved lsof vnode.
  allowed_real="$(realpath "$ALLOWED_PROD_EXE" 2>/dev/null || printf '%s' "$ALLOWED_PROD_EXE")"
  while IFS= read -r pid; do
    [[ -n "$pid" ]] || continue
    exe="$(pid_exe_path "$pid" || true)"
    exe_real="$(realpath "$exe" 2>/dev/null || printf '%s' "$exe")"
    if [[ -n "$exe" && "$exe_real" == "$allowed_real" ]]; then
      dmg_running=1
    elif pid_gone "$pid"; then
      : # process exited between pgrep and resolution — no longer running, ignore
    else
      # Alive (or indeterminate) and not the allowed DMG path: dev build,
      # unresolvable exe, or a liveness probe we can't trust — fail closed.
      dev_blocking+=("$pid:${exe:-<unresolved>}")
    fi
  done <<< "$running_pids"
fi
if [[ ${#dev_blocking[@]} -gt 0 ]]; then
  warn "A non-installed buzz-desktop process is running (dev build or unresolvable):"
  for entry in "${dev_blocking[@]}"; do warn "  PID ${entry%%:*} → ${entry#*:}"; done
  warn "Quit any running dev build, then re-run. This script never kills processes."
  exit 1
fi
if [[ $dmg_running -eq 1 ]]; then
  info "installed DMG is running — allowed (read-only source)"
  warn "Do NOT create/archive agents or edit teams in the DMG while this runs; the 5-file bundle is read as one snapshot."
else
  info "no running Buzz process detected"
fi

# 1b. On-disk symlink refusals. The paths are fixed canonical constants, so
#     there is no override spelling to canonicalize — but if something odd
#     already exists at those paths (a symlinked dev store, or a prod source
#     aliased elsewhere), following it would break the read-only promise or
#     write outside the dev dir. Reject symlinks before any mkdir/write.
[[ -d "$PROD_DIR" ]] || die "prod app-data dir not found: $PROD_DIR"
# Prod root must be a real directory, never a symlink: a symlinked prod root
# could alias the dev tree, making source and destination the same path, so the
# read-only prod promise would break and the commit could destroy prod data.
[[ -L "$PROD_DIR" ]] && die "refusing: prod app-data dir is a symlink; must be a real directory."
# Dev write targets: mkdir -p / the directory exchange would otherwise follow a
# symlink here and write outside the dev dir.
for d in "$DEV_DIR" "$DEV_DIR/agents"; do
  [[ -L "$d" ]] && die "refusing: $d is a symlink; write target must be a real directory."
done
# Prod source components: a symlinked prod/agents, record file, or teams/ dir
# could alias a dev target and be mutated or destroyed by the commit. Source
# must be real files/dirs. (Symlinked prod sources are also copied by value into
# the stage, silently importing whatever they point at — refuse instead.)
[[ -L "$PROD_DIR/agents" ]] && die "refusing: prod agents/ is a symlink; source must be a real directory."
for f in "${RECORD_FILES[@]}"; do
  [[ -L "$PROD_DIR/agents/$f" ]] \
    && die "refusing: prod agents/$f is a symlink; source must be a real file."
done
[[ -L "$PROD_DIR/agents/teams" ]] \
  && die "refusing: prod agents/teams is a symlink; source must be a real directory."
# Physical distinctness: prod and dev must not resolve to the same tree or one
# inside the other. Both roots are now confirmed non-symlinks, but resolve
# physically anyway to catch any alias in an ancestor component. Dev is
# normally absent post-reset (fixed sibling constants can't overlap); only
# compare when it already exists (a forced re-run).
prod_phys="$(cd "$PROD_DIR" && pwd -P)"
if [[ -d "$DEV_DIR" ]]; then
  dev_phys="$(cd "$DEV_DIR" && pwd -P)"
  [[ "$prod_phys" == "$dev_phys" || "$prod_phys" == "$dev_phys"/* || "$dev_phys" == "$prod_phys"/* ]] \
    && die "refusing: prod and dev resolve to the same or overlapping directory (prod=$prod_phys dev=$dev_phys)."
fi
info "path checks OK (prod=$PROD_DIR dev=$DEV_DIR)"

# 1c. Prod store must exist and parse.
prod_agents="$PROD_DIR/agents/managed-agents.json"
[[ -f "$prod_agents" ]]   || die "prod managed-agents.json not found: $prod_agents"
if have jq; then
  jq empty "$prod_agents" 2>/dev/null || die "prod managed-agents.json is not valid JSON: $prod_agents"
elif have python3; then
  python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$prod_agents" \
    || die "prod managed-agents.json is not valid JSON: $prod_agents"
fi
info "prod store OK: $prod_agents"

# 1d. Dev store must be absent/empty unless --force.
dev_agents="$DEV_DIR/agents/managed-agents.json"
if [[ -s "$dev_agents" ]]; then
  if [[ $FORCE -eq 1 ]]; then
    warn "dev store is non-empty; --force given, will overwrite: $dev_agents"
  else
    die "dev store already populated ($dev_agents). Re-run with --force to overwrite."
  fi
else
  info "dev store absent/empty: $dev_agents"
fi

# 1e. When a live agents/ already exists, the commit is an atomic directory
#     exchange (renamex_np RENAME_SWAP), which only python3 can drive. Require
#     it here so a missing interpreter fails preflight, never mid-commit.
if [[ $DRY_RUN -eq 0 && -d "$DEV_DIR/agents" ]] && ! have python3; then
  die "python3 is required to atomically replace the existing dev agents/ directory."
fi

# --- step 2: restore records (exact bundle, one atomic commit) --------------
# The five boot-critical paths (managed-agents.json, personas.json, teams.json,
# global-agent-config.json, teams/) are read as one related state, so the
# commit must be all-or-nothing.
# Phase 1 builds a COMPLETE replacement agents/ in a staging dir beside the live
# one — the prod bundle plus any unrelated live entries carried across verbatim,
# every transform validated, zero live-path mutation. Phase 2 commits it in ONE
# operation: a single rename when no live agents/ exists, otherwise one atomic
# renamex_np(RENAME_SWAP) exchange (fail closed, never a sequential fallback).
# A crash or I/O error therefore leaves agents/ either fully old or fully new.
say "[2/4] Agent records  prod → dev"
if [[ $DRY_RUN -eq 1 ]]; then
  info "[dry-run] stage a complete replacement agents/ beside the live dir"
  for f in "${RECORD_FILES[@]}"; do
    if [[ -f "$PROD_DIR/agents/$f" ]]; then
      if [[ "$f" == managed-agents.json || "$f" == personas.json ]]; then
        info "[dry-run] stage $f (strip runtime_pid, 0600)"
      else
        info "[dry-run] stage $f (0600)"
      fi
    elif [[ -e "$DEV_DIR/agents/$f" ]]; then
      info "[dry-run] drop stale dev $f (absent in prod)"
    else
      info "skip (absent in prod): $f"
    fi
  done
  if [[ -d "$PROD_DIR/agents/teams" ]]; then
    info "[dry-run] stage agents/teams/"
  elif [[ -e "$DEV_DIR/agents/teams" ]]; then
    info "[dry-run] drop stale dev agents/teams/ (absent in prod)"
  else
    info "skip (absent in prod): agents/teams/"
  fi
  if [[ -d "$DEV_DIR/agents" ]]; then
    info "[dry-run] commit via atomic directory exchange (existing dev agents/)"
  else
    info "[dry-run] commit via single rename (no existing dev agents/)"
  fi
else
  # Phase 1 — build the complete replacement in a staging dir on the dev
  # filesystem. mktemp -d creates it 0700; record files are chmod 0600. Only
  # $DEV_DIR (the stage parent) is created live — never agents/ — so a phase-1
  # failure leaves no live agents/ scaffold behind.
  mkdir -p "$DEV_DIR"
  stage_agents="$(mktemp -d "$DEV_DIR/agents.stage.XXXXXX")"
  STAGE_TEMPS+=("$stage_agents")

  # Seed the stage with the entire live agents/ (a single checked copy — under
  # `set -e` a partial copy aborts before any commit), then drop the five bundle
  # names FROM THE STAGE so the prod overlay below replaces them cleanly. Any
  # other live entry (logs/, agent-pids/, retention stores, …) is carried across
  # verbatim and preserved by the directory swap.
  if [[ -d "$DEV_DIR/agents" ]]; then
    cp -Rp "$DEV_DIR/agents/." "$stage_agents/"
    rm -rf -- \
      "$stage_agents/managed-agents.json" \
      "$stage_agents/personas.json" \
      "$stage_agents/teams.json" \
      "$stage_agents/global-agent-config.json" \
      "$stage_agents/teams"
  fi

  # Stage the prod bundle. A record file absent in prod is simply not staged, so
  # the swap drops any stale dev copy; the same holds for teams/.
  for f in "${RECORD_FILES[@]}"; do
    src="$PROD_DIR/agents/$f"
    if [[ -f "$src" ]]; then
      if [[ "$f" == managed-agents.json || "$f" == personas.json ]]; then
        strip_runtime_pid "$src" "$stage_agents/$f" || die "failed to transform $f (dev unchanged)"
        info "staged $f (runtime_pid stripped, mode 0600)"
      else
        cat "$src" > "$stage_agents/$f"
        info "staged $f (mode 0600)"
      fi
      chmod 600 "$stage_agents/$f"
    elif [[ -e "$DEV_DIR/agents/$f" ]]; then
      info "dropping stale dev $f (absent in prod)"
    else
      info "skip (absent in prod): $f"
    fi
  done
  if [[ -d "$PROD_DIR/agents/teams" ]]; then
    cp -Rp "$PROD_DIR/agents/teams" "$stage_agents/teams"
    info "staged agents/teams/"
  elif [[ -e "$DEV_DIR/agents/teams" ]]; then
    info "dropping stale dev agents/teams/ (absent in prod)"
  else
    info "skip (absent in prod): agents/teams/"
  fi

  # Phase 2 — commit the complete bundle in one atomic operation.
  if [[ -d "$DEV_DIR/agents" ]]; then
    swap_dirs "$stage_agents" "$DEV_DIR/agents" \
      || die "atomic directory exchange failed (dev agents/ unchanged)."
    rm -rf "$stage_agents"   # holds the swapped-out old tree after the exchange
    info "committed agents/ via atomic exchange"
  else
    mv "$stage_agents" "$DEV_DIR/agents"   # no live dir → single rename
    info "committed agents/ via rename (no prior dev store)"
  fi
fi

# --- step 3: owner identity → dev identity.key ------------------------------
say "[3/4] Owner identity  prod keychain → dev identity.key"
info "reading prod keychain (service=$KEYCHAIN_SVC account=$KEYCHAIN_ACCT) — one keychain prompt expected"
blob="$(security find-generic-password -s "$KEYCHAIN_SVC" -a "$KEYCHAIN_ACCT" -w 2>/dev/null || true)"
if [[ -z "$blob" ]]; then
  warn "no keychain entry for service=$KEYCHAIN_SVC account=$KEYCHAIN_ACCT — skipping owner identity."
  warn "Records were still copied. Your dev build will mint a fresh owner key on boot."
else
  # The blob is a JSON map; the owner nsec lives under the "identity" key and
  # MUST be a non-empty JSON string (a number/object/null would be a corrupt
  # key that boot rejects, falling through to a fresh identity — the exact
  # split-brain this helper prevents).
  if have jq; then
    identity="$(printf '%s' "$blob" | jq -er 'if (.identity | type) == "string" and (.identity | length) > 0 then .identity else empty end' 2>/dev/null || true)"
  else
    identity="$(printf '%s' "$blob" | python3 -c 'import json,sys; v=json.load(sys.stdin).get("identity"); print(v if isinstance(v,str) and v else "")' 2>/dev/null || true)"
  fi
  if [[ -z "$identity" ]]; then
    warn "keychain blob has no non-empty string \"identity\" entry — skipping owner identity (records still restored)."
  else
    keyfile="$DEV_DIR/identity.key"
    if [[ $DRY_RUN -eq 1 ]]; then
      info "[dry-run] write owner nsec → $keyfile (mode 0600, atomic) [secret not shown]"
    else
      mkdir -p "$DEV_DIR"
      # Stage to a 0600 temp, then atomic rename — never truncate the live key
      # path, so an interruption cannot leave a partial/corrupt identity.key.
      stage="$(new_stage "$keyfile")"
      STAGE_TEMPS+=("$stage")
      printf '%s' "$identity" > "$stage"
      finalize "$stage" "$keyfile"
      info "wrote $keyfile (mode 0600) — dev boot adopts it into the dev keyring, then deletes it"
    fi
  fi
fi

# --- step 4: closing checklist ----------------------------------------------
say "[4/4] Done. Next steps"
cat <<EOF
  1. Start your dev build (\`just production\` or \`just dev\`).
     - First boot re-runs the agent-key migration → ONE prod-keychain prompt
       (copies agent:<pubkey> keys buzz-desktop → buzz-desktop-dev).
     - First boot adopts identity.key into the dev keyring, then deletes it.
  2. Worktree launches: worktree-suffixed dev dirs are symlinked to the
     canonical dev dir by sync_shared_agent_data ONLY when BUZZ_SHARE_IDENTITY=1.
     If you launch from a worktree, export BUZZ_SHARE_IDENTITY=1 (and
     BUZZ_PRIVATE_KEY) or the worktree will mint its own duplicate agents.
EOF
if [[ "${BUZZ_SHARE_IDENTITY:-}" != "1" ]]; then
  say "  NOTE: BUZZ_SHARE_IDENTITY is not set in this shell — set it for worktree launches."
fi
exit 0
