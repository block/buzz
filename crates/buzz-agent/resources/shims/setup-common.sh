#!/bin/bash
# Shared bootstrap for MCP toolchain shims.
# Sources hermit-managed uv/node into PATH with config isolation.
# Usage: source setup-common.sh <toolchain>   (toolchain = "uv" or "node")

set -euo pipefail

SHIM_TOOLCHAIN="${1:?usage: source setup-common.sh <uv|node>}"
SHIM_NAME="$(basename "${BASH_SOURCE[1]:-$0}")"

# ── Logging ──────────────────────────────────────────────────────────────────

# Declared here so shellcheck (SC2154) sees it as defined before the ERR
# traps below assign it on every exit path.
rc=0

_shim_log() {
    local msg
    msg="$(date +'%Y-%m-%d %H:%M:%S') [${SHIM_NAME}] $1"
    echo "${msg}" >&2
    if [ -n "${BUZZ_MCP_LOG_DIR:-}" ]; then
        (umask 077; echo "${msg}" >> "${BUZZ_MCP_LOG_DIR}/mcp-shim.log") 2>/dev/null || true
    fi
}

trap 'rc=$?; _shim_log "error: exiting with status ${rc}"; exit "${rc}"' ERR

# ── Config directory ─────────────────────────────────────────────────────────

BUZZ_MCP_HERMIT_DIR="${BUZZ_MCP_HERMIT_DIR:-${HOME}/.config/buzz/mcp-hermit}"
export BUZZ_MCP_HERMIT_DIR
mkdir -p "${BUZZ_MCP_HERMIT_DIR}"

# ── Lock (mkdir-based mutex with atomic dead-owner reclaim) ──────────────────
#
# Invariants:
# (a) No process ever removes the shared lock path based on a prior
#     observation — a stale "is dead" check followed by rm would let a
#     second waiter delete a freshly-reacquired live lock (TOCTOU).
# (b) Any removed name is either owned by the remover or atomically claimed
#     at the moment of removal.
#
# Protocol: try `mkdir _LOCK_DIR` (atomic). On failure, read the holder
# info and prove death. If dead, atomically `mv _LOCK_DIR` to a
# process-specific reclaim path — `mv` on the same filesystem is atomic,
# so only one of N racing reclaimers succeeds (the rest get ENOENT and
# retry). The winner removes only its own reclaim dir, then retries the
# `mkdir`. A live holder is never evicted; the outer MCP init timeout
# (BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS, default 300s) bounds the wait.
#
# `kill -0` returning ESRCH proves the PID is gone. On Linux, /proc-based
# starttime comparison additionally detects PID reuse. On macOS (same-user
# desktop app), all shim processes run as the same user, so EPERM from
# `kill -0` is not possible — a failure always means the process is dead.

_LOCK_DIR="${BUZZ_MCP_HERMIT_DIR}/.mcp-hermit-setup.lock"

_proc_starttime() {
    local pid="$1"
    local stat_file="/proc/${pid}/stat"
    local line rest
    [ -r "${stat_file}" ] || return 0
    line="$(cat "${stat_file}" 2>/dev/null)" || return 0
    rest="${line##*) }"
    # shellcheck disable=SC2086 # intentional word-splitting to index stat fields
    set -- $rest
    echo "${20:-}"
}

_lock_holder_is_dead() {
    local pid="$1" recorded_starttime="$2"
    [ -n "${pid}" ] || return 1
    if ! kill -0 "${pid}" 2>/dev/null; then
        # On Linux, corroborate via /proc — kill -0 failure could be
        # EPERM if processes ever run under different users (not the case
        # today: same-user desktop app). If /proc exists but the pid dir
        # is absent, the process is truly gone.
        if [ -d "/proc" ] && [ -d "/proc/${pid}" ]; then
            return 1 # /proc says alive — kill failed for another reason
        fi
        return 0
    fi
    if [ "$(uname -s)" = "Linux" ]; then
        local current_starttime
        current_starttime="$(_proc_starttime "${pid}")"
        if [ -n "${recorded_starttime}" ] && [ -n "${current_starttime}" ] \
            && [ "${recorded_starttime}" != "${current_starttime}" ]; then
            return 0
        fi
    fi
    return 1
}

while ! mkdir "${_LOCK_DIR}" 2>/dev/null; do
    if [ -f "${_LOCK_DIR}/info" ]; then
        _holder_pid=$(cut -d: -f1 "${_LOCK_DIR}/info" 2>/dev/null || echo "")
        _holder_starttime=$(cut -d: -f2 "${_LOCK_DIR}/info" 2>/dev/null || echo "")
        if _lock_holder_is_dead "${_holder_pid}" "${_holder_starttime}"; then
            # Atomic reclaim: mv to a process-specific path. Only one racer
            # succeeds; losers get ENOENT and loop back to retry mkdir.
            _reclaim_dir="${_LOCK_DIR}.reclaim.$$"
            if mv "${_LOCK_DIR}" "${_reclaim_dir}" 2>/dev/null; then
                _shim_log "dead lock holder detected (pid ${_holder_pid}); reclaimed"
                rm -rf "${_reclaim_dir}"
            fi
        fi
    fi
    sleep 0.1
done

echo "$$:$(_proc_starttime "$$"):$(date +%s)" > "${_LOCK_DIR}/info"
trap 'rc=$?; rm -rf "${_LOCK_DIR}"; _shim_log "error: exiting with status ${rc}"; exit "${rc}"' ERR
trap 'rm -rf "${_LOCK_DIR}"' EXIT

# ── macOS PATH fix ───────────────────────────────────────────────────────────
# GUI-launched macOS apps inherit a minimal PATH that omits sbin directories;
# hermit bootstrap needs chown from /usr/sbin.
export PATH="/usr/sbin:/sbin:${PATH}"

# ── Hermit bootstrap (SHA-verified, validated, atomically published) ───────

mkdir -p "${BUZZ_MCP_HERMIT_DIR}/bin"
cd "${BUZZ_MCP_HERMIT_DIR}"

mkdir -p "${BUZZ_MCP_HERMIT_DIR}/cache"
export HERMIT_STATE_DIR="${BUZZ_MCP_HERMIT_DIR}/cache"

HERMIT_BIN="${BUZZ_MCP_HERMIT_DIR}/bin/hermit"

_hermit_binary_valid() {
    # Side-effect-free: `--version` only prints the CLI's own version, it
    # touches no state/cache/network (github.com/cashapp/hermit's
    # `cmd/hermit/builtin/hermit.hcl` uses the same invocation as its own
    # smoke test). Guards against a truncated/corrupt/non-executable
    # artifact left by an interrupted prior run.
    [ -x "$1" ] && "$1" --version >/dev/null 2>&1
}

if ! _hermit_binary_valid "${HERMIT_BIN}"; then
    if [ -e "${HERMIT_BIN}" ]; then
        _shim_log "existing hermit binary invalid; removing and rebootstrapping"
        rm -f "${HERMIT_BIN}"
    fi
    _shim_log "downloading hermit binary (SHA-verified)"
    HERMIT_DIST_URL="https://github.com/cashapp/hermit/releases/download/stable"
    INSTALL_SCRIPT_SHA256="09ed936378857886fd4a7a4878c0f0c7e3d839883f39ca8b4f2f242e3126e1c6"

    _install_tmp="$(mktemp)"
    trap 'rc=$?; rm -f "${_install_tmp}"; rm -rf "${_LOCK_DIR}"; _shim_log "error: exiting with status ${rc}"; exit "${rc}"' ERR

    curl -fsSL "${HERMIT_DIST_URL}/install-${INSTALL_SCRIPT_SHA256}.sh" -o "${_install_tmp}"
    _actual_sha=$(openssl dgst -sha256 "${_install_tmp}" | awk '{print $2}')
    if [ "${_actual_sha}" != "${INSTALL_SCRIPT_SHA256}" ]; then
        rm -f "${_install_tmp}"
        _shim_log "FATAL: hermit install script SHA mismatch: got ${_actual_sha}"
        exit 1
    fi

    # The pinned installer writes to $HERMIT_EXE (default
    # $HERMIT_STATE_DIR/pkg/hermit@<channel>/hermit, NOT bin/hermit) and, if
    # not skipped, also drops a launcher stub under a system-wide install
    # dir like ~/bin. Point it at our own staging path and skip that
    # system-wide step entirely so bootstrap stays confined to
    # BUZZ_MCP_HERMIT_DIR and publication is a single atomic rename.
    _hermit_staged="${HERMIT_BIN}.download.$$"
    HERMIT_EXE="${_hermit_staged}" HERMIT_SKIP_SYSTEM_INSTALL=1 \
        /bin/bash "${_install_tmp}" 1>&2
    rm -f "${_install_tmp}"

    if ! _hermit_binary_valid "${_hermit_staged}"; then
        rm -f "${_hermit_staged}"
        _shim_log "FATAL: downloaded hermit binary failed validation"
        exit 1
    fi
    mv -f "${_hermit_staged}" "${HERMIT_BIN}"
    _shim_log "hermit bootstrap complete"
fi

export PATH="${BUZZ_MCP_HERMIT_DIR}/bin:${PATH}"

# ── Hermit init + activation (validated, with repair) ────────────────────────
#
# Real hermit's activate-hermit sources `hermit activate`, which exports
# HERMIT_ENV. A malformed/hostile/stale file could `exit 0` from the
# sourcing shell — silently killing the whole script — so we probe in a
# throwaway subshell first. If the probe fails (missing, malformed, or
# stale activate-hermit), we repair once (rm + re-init under the
# still-held lock) and FATAL on second failure. Trust-on-existence is dead:
# the file must exist AND produce HERMIT_ENV in a subshell.

_activation_valid() {
    [ -f "bin/activate-hermit" ] || return 1
    # Probe in a subshell: source the file, then write a sentinel if
    # HERMIT_ENV got set. A malformed file that `exit`s kills only the
    # subshell — the sentinel never appears, so the probe returns failure.
    local _probe_out
    _probe_out="$(mktemp)"
    ( . "bin/activate-hermit" >/dev/null 2>&1; [ -n "${HERMIT_ENV:-}" ] && echo ok > "${_probe_out}" ) 2>/dev/null
    local _result=1
    [ -s "${_probe_out}" ] && _result=0
    rm -f "${_probe_out}"
    return "${_result}"
}

_run_hermit_init() {
    # Linux: copy hermit binary to a private temp dir to avoid self-update
    # lock contention, and invoke it by absolute path so a hostile
    # pre-existing entry earlier in PATH can never be executed instead.
    if [ "$(uname -s)" = "Linux" ]; then
        _htmp_cleanup="$(mktemp -d "${TMPDIR:-/tmp}/buzz-hermit.XXXXXXXX")"
        trap 'rc=$?; rm -rf "${_htmp_cleanup}"; rm -rf "${_LOCK_DIR}"; _shim_log "error: exiting with status ${rc}"; exit "${rc}"' ERR
        _htmp_hermit="${_htmp_cleanup}/hermit"
        cp "${HERMIT_BIN}" "${_htmp_hermit}"
        chmod +x "${_htmp_hermit}"
        "${_htmp_hermit}" init 1>&2
        rm -rf "${_htmp_cleanup}"
    else
        hermit init 1>&2
    fi
}

_init_attempt=0
while ! _activation_valid; do
    if [ "${_init_attempt}" -ge 2 ]; then
        _shim_log "FATAL: bin/activate-hermit still invalid after re-init — hermit environment is broken"
        exit 1
    fi
    if [ "${_init_attempt}" -eq 0 ]; then
        _shim_log "initializing hermit environment"
    else
        _shim_log "bin/activate-hermit invalid after init; repairing (re-init under lock)"
    fi
    rm -f "bin/activate-hermit"
    _run_hermit_init
    _init_attempt=$((_init_attempt + 1))
done

_shim_log "activating hermit environment"
{ . "bin/activate-hermit"; } 1>&2 2>/dev/null

# Install the requested toolchain packages.
case "${SHIM_TOOLCHAIN}" in
    uv)
        hermit install python3@3.10 1>&2
        hermit install uv 1>&2
        ;;
    node)
        hermit install node 1>&2
        # Create distinct empty npmrc files for config isolation.
        touch "${BUZZ_MCP_HERMIT_DIR}/empty-user-npmrc"
        touch "${BUZZ_MCP_HERMIT_DIR}/empty-global-npmrc"
        ;;
    *)
        _shim_log "unknown toolchain: ${SHIM_TOOLCHAIN}"
        exit 1
        ;;
esac

# ── Release lock ─────────────────────────────────────────────────────────────

rm -rf "${_LOCK_DIR}"
trap 'rc=$?; _shim_log "error: exiting with status ${rc}"; exit "${rc}"' ERR
trap - EXIT

_shim_log "bootstrap complete for ${SHIM_TOOLCHAIN}"
