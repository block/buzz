#!/bin/sh
# =============================================================================
# provision-buzz-host.sh — preflight a Linux host for Buzz remote agents
# =============================================================================
# Usage:
#   ./scripts/provision-buzz-host.sh     # run ON the host, as the agent user
#
# Checks (and where it can, fixes) the host contract that `buzz-backend-ssh`
# assumes: see docs/remote-agents.md, "Host prerequisites". Safe to re-run —
# every action is idempotent, and a fully provisioned host is a no-op.
#
# It installs nothing itself. Harness CLIs have their own installers and their
# own authentication, and stay an operator step. The two Buzz tools — `buzz-acp`
# and the `buzz` CLI — the deploy op resolves on the host's PATH or in
# ~/.local/bin, and *installs* when it resolves none and the desktop supplied a
# binary to push (`BUZZ_ACP_PUSH_BINARY` / `BUZZ_CLI_PUSH_BINARY`, see
# docs/remote-agents.md). With no binary supplied, a missing `buzz-acp` fails
# the deploy with exit 90 — this preflight is what you run first — while a
# missing `buzz` CLI only warns and the deploy continues.
#
# Exit 0 when the mandatory set is green (lingering, ~/.local/bin, systemd
# --user); 1 otherwise. Everything else is reported as a note, never a failure.
# =============================================================================
set -eu

USER_NAME="$(id -un)"
HOME_DIR="${HOME:-$(cd ~ && pwd)}"
LOCAL_BIN="${HOME_DIR}/.local/bin"

# Summary rows, one per line, `requirement|status|action`. POSIX sh has no
# arrays, and a newline-delimited string prints back through one `read` loop.
ROWS=""
BLOCKERS=0

# `mandatory` as the third positional: a red row there is what decides the exit
# code, so the caller cannot forget to account for one.
add_row() { # requirement, status, action, [mandatory]
  ROWS="${ROWS}${1}|${2}|${3}
"
  if [ "${4:-}" = "mandatory" ] && [ "${2}" != "OK" ]; then
    BLOCKERS=$((BLOCKERS + 1))
  fi
}

note() { printf '%s\n' "$*"; }

# ---- 1. The agent user ------------------------------------------------------
# The whole flow is root-free: the env file holding the minted nsec lands under
# this user's ownership, beside the harness credentials that already live in
# its home (~/.claude, ~/.config/goose). Provisioning as root would create
# those paths for the wrong user and the deploy would silently target another.
if [ "$(id -u)" -eq 0 ]; then
  note "provision-buzz-host: refusing to run as root."
  note "  Run this as the unprivileged user the agents will run as, e.g.:"
  note "    su - ubuntu -c '/path/to/provision-buzz-host.sh'"
  exit 1
fi
add_row "non-root user" "OK" "running as ${USER_NAME}"

# ---- 2. Lingering -----------------------------------------------------------
# THE non-obvious prerequisite. Without it the user manager is torn down when
# the last session ends, so the agent is killed the moment the deploy's own SSH
# session closes — which reads as a flaky agent, not as a misconfiguration. It
# also creates /run/user/$(id -u), without which no `systemctl --user` call can
# reach the bus at all.
linger_is_on() {
  # Two sources because either can be unavailable: `loginctl` needs a logind
  # user record (absent on a host with no active session), the marker file is
  # world-readable and always authoritative.
  if [ -e "/var/lib/systemd/linger/${USER_NAME}" ]; then
    return 0
  fi
  loginctl show-user "${USER_NAME}" --property=Linger 2>/dev/null |
    grep -q '^Linger=yes$'
}

if linger_is_on; then
  add_row "loginctl linger" "OK" "already enabled" mandatory
else
  # Best-effort, exactly as the deploy script's own call is: some hosts gate
  # enable-linger behind polkit, where it needs a root run instead.
  loginctl enable-linger "${USER_NAME}" >/dev/null 2>&1 || true
  if linger_is_on; then
    add_row "loginctl linger" "OK" "enabled just now" mandatory
  else
    add_row "loginctl linger" "MISSING" \
      "run: sudo loginctl enable-linger ${USER_NAME}" mandatory
  fi
fi

# ---- 3. ~/.local/bin --------------------------------------------------------
# Where `buzz-acp` and most harness CLIs install, and what the deploy prepends
# to the unit's PATH. It has to exist and be on the login PATH so `command -v`
# resolves the same binaries the unit will.
mkdir -p "${LOCAL_BIN}"

case ":${PATH}:" in
  *":${LOCAL_BIN}:"*)
    add_row "local bin on PATH" "OK" "${LOCAL_BIN}" mandatory
    ;;
  *)
    # Append only when absent: re-running must not stack duplicate exports into
    # a file the user also edits by hand.
    if [ -f "${HOME_DIR}/.profile" ] &&
      grep -q '\.local/bin' "${HOME_DIR}/.profile"; then
      add_row "local bin on PATH" "NOTE" \
        "already in ~/.profile; re-login to pick it up" mandatory
    else
      # Single quotes deliberately: $HOME and $PATH must reach .profile as
      # literals, to be expanded at each login rather than frozen now.
      # shellcheck disable=SC2016
      printf '\n# Added by provision-buzz-host.sh\nexport PATH="$HOME/.local/bin:$PATH"\n' \
        >>"${HOME_DIR}/.profile"
      add_row "local bin on PATH" "NOTE" \
        "appended to ~/.profile; re-login to pick it up" mandatory
    fi
    ;;
esac

# ---- 4. buzz-acp ------------------------------------------------------------
# Reported, never installed. `discover_harnesses` tolerates its absence, but
# `deploy` refuses with exit 90, so a host that stops here fails late.
if command -v buzz-acp >/dev/null 2>&1; then
  add_row "buzz-acp" "OK" "$(command -v buzz-acp)"
else
  add_row "buzz-acp" "MISSING" "copy the release binary to ${LOCAL_BIN}/buzz-acp"
fi

# ---- 5. buzz CLI ------------------------------------------------------------
# Reported, never installed — and never mandatory. Deploy pushes it when the
# desktop supplies one, and without it the deploy only warns: agents cannot
# reply with `buzz messages send` and degrade to slower replies.
if command -v buzz >/dev/null 2>&1; then
  add_row "buzz" "OK" "$(command -v buzz)"
else
  add_row "buzz" "MISSING" \
    "deploy will push it, or copy the release binary to ${LOCAL_BIN}/buzz"
fi

# ---- 6. Harness CLIs --------------------------------------------------------
# `discover_harnesses` probes the ACP ADAPTER name, not the vendor CLI, and the
# adapter is what the deploy pins — but the adapter is a shim over the vendor
# CLI, which carries the authentication. Both must be present, so both are
# reported. Neither is installed here: each has its own installer and its own
# interactive login, which cannot run over a non-interactive SSH deploy.
check_harness() { # label, adapter command, vendor cli, install hint
  _adapter="$(command -v "$2" 2>/dev/null || true)"
  _cli="$(command -v "$3" 2>/dev/null || true)"
  if [ -n "${_adapter}" ] && [ -n "${_cli}" ]; then
    add_row "harness: $1" "OK" "${_adapter}"
  elif [ -n "${_cli}" ]; then
    add_row "harness: $1" "NOTE" "$3 present, ACP adapter $2 missing — $4"
  elif [ -n "${_adapter}" ]; then
    add_row "harness: $1" "NOTE" "$2 present, $3 CLI missing — install and log in"
  else
    add_row "harness: $1" "MISSING" "$4"
  fi
}

CLAUDE_ACP_ADAPTER="claude-agent-acp"
if ! command -v "${CLAUDE_ACP_ADAPTER}" >/dev/null 2>&1 &&
  command -v claude-code-acp >/dev/null 2>&1; then
  CLAUDE_ACP_ADAPTER="claude-code-acp"
fi
check_harness "claude" "${CLAUDE_ACP_ADAPTER}" "claude" \
  "npm i -g @agentclientprotocol/claude-agent-acp; curl -fsSL https://claude.ai/install.sh | bash"
check_harness "codex" "codex-acp" "codex" \
  "npm i -g @agentclientprotocol/codex-acp; curl -fsSL https://chatgpt.com/codex/install.sh | sh"

# ---- 7. Tailscale (optional) ------------------------------------------------
# An enhancement, never a dependency: absence only costs the desktop's device
# picker, and manual SSH is the unchanged fallback. So every branch here is a
# note.
if command -v tailscale >/dev/null 2>&1; then
  # jq-free on purpose: this script must run on a bare host, and jq is not a
  # prerequisite of anything else in the contract. BackendState is top-level and
  # appears once, so the first match is the right one.
  TS_STATE="$(tailscale status --json 2>/dev/null |
    sed -n 's/.*"BackendState"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
    head -n 1)"
  [ -n "${TS_STATE}" ] || TS_STATE="unknown"

  if [ "${TS_STATE}" = "Running" ]; then
    # SSH-server state comes from prefs, not from Self.sshHostKeys in the status
    # document: that field is populated for PEERS (it is how the desktop marks a
    # device "· Tailscale SSH"), and reads as null when a node looks at itself —
    # so trusting it here reports a false negative on an SSH-enabled host.
    if tailscale debug prefs 2>/dev/null |
      grep -q '"RunSSH"[[:space:]]*:[[:space:]]*true'; then
      add_row "tailscale" "OK" "Running, Tailscale SSH enabled"
    else
      add_row "tailscale" "NOTE" \
        "Running, no SSH server — run: tailscale set --ssh (or tailscale up --ssh)"
    fi
  else
    add_row "tailscale" "NOTE" \
      "BackendState=${TS_STATE} — run: tailscale up (optional)"
  fi
else
  add_row "tailscale" "NOTE" "not installed (optional; use plain SSH keys)"
fi

# ---- 8. systemd --user ------------------------------------------------------
# The deploy's own workaround, mirrored: a non-interactive SSH command often
# gets no XDG_RUNTIME_DIR, and without it every `systemctl --user` fails with
# "Failed to connect to bus". Checking under the same assumption is the point —
# a check that only passes in a login shell would pass on hosts the deploy then
# fails on.
if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
  XDG_RUNTIME_DIR="/run/user/$(id -u)"
  export XDG_RUNTIME_DIR
fi

if systemctl --user show-environment >/dev/null 2>&1; then
  add_row "systemd --user" "OK" "bus reachable at ${XDG_RUNTIME_DIR}" mandatory
else
  add_row "systemd --user" "MISSING" \
    "no user bus at ${XDG_RUNTIME_DIR} — enable lingering, then re-run" mandatory
fi

# ---- Summary ----------------------------------------------------------------

# `hostname` is not in every minimal image's base install, so it never decides
# an exit code — only how the header reads.
HOST_LABEL="$(hostname 2>/dev/null || echo "this host")"
printf '\n%s\n' "Buzz remote-agent host preflight — ${USER_NAME}@${HOST_LABEL}"
printf '\n  %-22s %-8s %s\n' "REQUIREMENT" "STATUS" "ACTION / DETAIL"
printf '  %-22s %-8s %s\n' "----------------------" "--------" "---------------"
printf '%s' "${ROWS}" | while IFS='|' read -r req status action; do
  [ -n "${req}" ] || continue
  printf '  %-22s %-8s %s\n' "${req}" "${status}" "${action}"
done
printf '\n'

if [ "${BLOCKERS}" -eq 0 ]; then
  note "Mandatory checks green. Deploy a remote agent from the Buzz desktop app."
  note "Reminder: buzz-acp, the buzz CLI and the harness CLIs are installed separately."
  exit 0
fi

note "${BLOCKERS} mandatory check(s) not satisfied — see ACTION above."
exit 1
