#!/usr/bin/env bash
# Chunk 03 -- host DNS + CA trust. Runs on YOUR OWN COMPUTER, never in the guest.
#
# Two jobs, deliberately separable:
#
#   1. Point 'buzz-vm.test' and 'admin.buzz-vm.test' at 127.0.0.1 in the host's
#      hosts file, where the VM's NAT forwards (8080 -> 80, 8443 -> 443) are
#      waiting. SOP Step 10.
#   2. Copy Caddy's 'tls internal' root CA out of the guest so it can be trusted
#      on this machine. hardening-spec.md §A.3.
#
# Job 2 has a hard ordering dependency: the CA does not exist until Caddy has
# started at least once (chunk 07), so job 2 is OPT-IN. Default is job 1 only.
#
# This script NEVER installs the CA into a trust store. It prints the exact
# command and stops -- adding a root CA to the System keychain is the operator's
# decision, and a script that makes it silently is a script that can be reused
# against the wrong machine.
#
# Exit codes:
#   0  done, nothing left to do
#   1  usage error, or manual intervention required (Windows hosts file, or a
#      name already mapped to the wrong address)
#   2  VM unreachable over ssh
#   3  no running Caddy container -- chunk 07 has not run yet
#   4  CA file missing inside the container, or not a PEM certificate
set -euo pipefail

# --- Canonical dev values. See hardening-spec.md §A.2 for the dev/prod table. ---
DOMAIN="${DOMAIN:-buzz-vm.test}"
ADMIN_DOMAIN="${ADMIN_DOMAIN:-admin.${DOMAIN}}"

# ssh into the guest. Port 2222 and user 'dev' come from build-vps-clone.sh.
# 'dev' rather than root because SOP Step 13.3 disables root ssh login, and this
# script must keep working after hardening.
VM_HOST="${VM_HOST:-127.0.0.1}"
VM_PORT="${VM_PORT:-2222}"
VM_USER="${VM_USER:-dev}"

# Caddy's local CA, inside the caddy container (the buzz-caddy-data volume).
CADDY_CA_PATH="${CADDY_CA_PATH:-/data/caddy/pki/authorities/local/root.crt}"
CA_DIR="${CA_DIR:-$HOME/vm-images}"
CA_OUT="${CA_OUT:-$CA_DIR/buzz-dev-caddy-root.crt}"

MARKER="# ${DOMAIN} -- added by launchpad/deploy/virtual-box/host-dns.sh"

usage() {
  cat <<'USAGE'
usage: host-dns.sh [--skip-ca | --ca | --ca-only] [--help]

  (no flags)   job 1 only: add the two hosts-file entries. Safe before the
               stack exists. This is the default because Caddy's CA does not
               exist until chunk 07.
  --skip-ca    explicit form of the default.
  --ca         job 1 and job 2: hosts entries, then fetch the Caddy root CA.
  --ca-only    job 2 only. No sudo needed. Use this to revisit after chunk 07.

environment overrides:
  DOMAIN ADMIN_DOMAIN HOSTS_FILE VM_HOST VM_PORT VM_USER CADDY_CA_PATH CA_OUT
USAGE
}

DO_DNS=1
DO_CA=0
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-ca) DO_CA=0 ;;
    --ca)      DO_CA=1 ;;
    --ca-only) DO_CA=1; DO_DNS=0 ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'host-dns.sh: unknown argument: %s\n\n' "$1" >&2; usage >&2; exit 1 ;;
  esac
  shift
done

say()  { printf '\n=== %s ===\n' "$1"; }
info() { printf '    %s\n' "$1"; }
warn() { printf '!!  %s\n' "$1" >&2; }

# --- Platform. Only the hosts-file path and the trust command differ. ---
case "$(uname -s)" in
  Darwin)               PLATFORM=macos   ; DEFAULT_HOSTS=/etc/hosts ;;
  MINGW*|MSYS*|CYGWIN*) PLATFORM=windows ; DEFAULT_HOSTS=/c/Windows/System32/drivers/etc/hosts ;;
  Linux)                PLATFORM=linux   ; DEFAULT_HOSTS=/etc/hosts ;;
  *) warn "unsupported host platform: $(uname -s). The SOP covers macOS and Git Bash on Windows."; exit 1 ;;
esac
HOSTS_FILE="${HOSTS_FILE:-$DEFAULT_HOSTS}"

# ---------------------------------------------------------------------------
# Job 1 -- hosts file
# ---------------------------------------------------------------------------

# Print every address the hosts file maps a given name to. Comment-aware, and
# strips the CR that the Windows file carries on every line.
hosts_addresses_for() {
  awk -v want="$1" '
    { sub(/\r$/, "") }
    { sub(/#.*/, "") }
    NF >= 2 { for (i = 2; i <= NF; i++) if ($i == want) print $1 }
  ' "$HOSTS_FILE"
}

# ok | missing | conflict
entry_state() {
  local addrs
  addrs="$(hosts_addresses_for "$1")"
  if [ -z "$addrs" ]; then
    echo missing
  elif printf '%s\n' "$addrs" | grep -qx '127\.0\.0\.1'; then
    echo ok
  else
    echo conflict
  fi
}

windows_manual_instructions() {
  cat <<EOF

Git Bash has no 'sudo' and cannot elevate itself, so this script will not
pretend to have edited the file. Do it by hand -- SOP Step 10:

  1. Press Start, type 'notepad'
  2. RIGHT-CLICK Notepad and choose "Run as administrator". Without this the
     save appears to work and is quietly discarded.
  3. File -> Open, and paste this into the filename box:
       C:\\Windows\\System32\\drivers\\etc\\hosts
  4. Add these lines at the end:
       127.0.0.1 ${DOMAIN}
       127.0.0.1 ${ADMIN_DOMAIN}
  5. File -> Save

Then re-run this script to confirm:
  ./host-dns.sh

EOF
}

do_dns() {
  say "Job 1: hosts entries in $HOSTS_FILE"

  if [ ! -f "$HOSTS_FILE" ]; then
    warn "hosts file not found: $HOSTS_FILE (override with HOSTS_FILE=...)"
    exit 1
  fi

  local missing="" conflicted="" name state
  for name in "$DOMAIN" "$ADMIN_DOMAIN"; do
    state="$(entry_state "$name")"
    case "$state" in
      ok)       info "$name -> 127.0.0.1 (already present)" ;;
      missing)  info "$name -> MISSING"; missing="$missing $name" ;;
      conflict) info "$name -> $(hosts_addresses_for "$name" | tr '\n' ' ')(WRONG ADDRESS)"
                conflicted="$conflicted $name" ;;
    esac
  done

  # A name already pointed somewhere else is not something to append past --
  # the resolver may pick either line and the failure looks like a relay bug.
  if [ -n "$conflicted" ]; then
    warn "already mapped to an address that is not 127.0.0.1:$conflicted"
    warn "edit $HOSTS_FILE by hand, remove those lines, then re-run."
    [ "$PLATFORM" = windows ] && windows_manual_instructions
    exit 1
  fi

  if [ -z "$missing" ]; then
    info "nothing to change."
    verify_resolution
    return 0
  fi

  if [ "$PLATFORM" = windows ]; then
    warn "cannot edit $HOSTS_FILE from Git Bash -- no sudo, no elevation."
    windows_manual_instructions
    exit 1
  fi

  # Build the block. The leading newline is load-bearing: a hosts file with no
  # trailing newline would otherwise get the first entry glued onto its last line.
  local block=""
  if ! grep -qF "$MARKER" "$HOSTS_FILE" 2>/dev/null; then
    block="${MARKER}"$'\n'
  fi
  for name in $missing; do
    block="${block}127.0.0.1 ${name}"$'\n'
  done

  info "appending to $HOSTS_FILE (sudo will ask for your login password):"
  printf '%s' "$block" | sed 's/^/      /'
  printf '\n%s' "$block" | sudo tee -a "$HOSTS_FILE" >/dev/null

  if [ "$PLATFORM" = macos ]; then
    # macOS caches negative lookups. Without this the browser can keep failing
    # for minutes after a correct edit.
    sudo dscacheutil -flushcache 2>/dev/null || true
    sudo killall -HUP mDNSResponder 2>/dev/null || true
    info "flushed the macOS DNS cache."
  fi

  verify_resolution
}

verify_resolution() {
  local name
  for name in "$DOMAIN" "$ADMIN_DOMAIN"; do
    if [ "$PLATFORM" = macos ]; then
      if dscacheutil -q host -a name "$name" 2>/dev/null | grep -q 'ip_address: 127.0.0.1'; then
        info "resolves: $name -> 127.0.0.1"
      else
        warn "does NOT resolve to 127.0.0.1: $name"
      fi
    elif command -v getent >/dev/null 2>&1; then
      if getent hosts "$name" 2>/dev/null | grep -q '^127\.0\.0\.1'; then
        info "resolves: $name -> 127.0.0.1"
      else
        warn "does NOT resolve to 127.0.0.1: $name"
      fi
    else
      info "resolution check skipped on this platform -- confirm in the browser."
    fi
  done
}

# ---------------------------------------------------------------------------
# Job 2 -- Caddy's local root CA
# ---------------------------------------------------------------------------

ssh_vm() {
  # ConnectTimeout so an absent VM fails in seconds instead of hanging. No
  # BatchMode: a passphrase-protected key must still be able to prompt.
  ssh -p "$VM_PORT" \
      -o ConnectTimeout=5 \
      "${VM_USER}@${VM_HOST}" "$@"
}

print_trust_command() {
  local path="$1"
  say "Trust it -- your decision, not this script's"
  case "$PLATFORM" in
    macos)
      cat <<EOF
Run this yourself if you want the browser and the desktop app to accept the
relay's certificate. It writes to the system-wide trust store and will ask for
your password:

  sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain $path

To undo it later:

  sudo security remove-trusted-cert -d $path
EOF
      ;;
    windows)
      cat <<EOF
Open a Command Prompt as Administrator and run:

  certutil -addstore -f "ROOT" "$(cygpath -w "$path" 2>/dev/null || printf '%s' "$path")"

To undo it, use certmgr.msc -> Trusted Root Certification Authorities and delete
the entry issued by Caddy Local Authority.
EOF
      ;;
    linux)
      cat <<EOF
  sudo cp $path /usr/local/share/ca-certificates/buzz-dev-caddy-root.crt
  sudo update-ca-certificates
EOF
      ;;
  esac
  cat <<EOF

Without trusting it: the browser shows a certificate warning you can click
through, and the desktop app -- a Rust/Tauri client with no "proceed anyway"
prompt -- refuses the wss:// connection outright. The second is the failure you
will actually hit.
EOF
}

do_ca() {
  say "Job 2: Caddy root CA from ${VM_USER}@${VM_HOST}:${VM_PORT}"

  if ! ssh_vm true 2>/dev/null; then
    warn "cannot ssh to ${VM_USER}@${VM_HOST} port ${VM_PORT}."
    cat >&2 <<EOF

Check, in this order:
  VBoxManage list runningvms                 # is buzz-dev up?
  ssh-keygen -R '[${VM_HOST}]:${VM_PORT}'    # host key changed by a VM rebuild
  ssh -p ${VM_PORT} ${VM_USER}@${VM_HOST}    # run it by hand to see the real error
EOF
    exit 2
  fi

  # Find the container by its Compose service label, not by name: the project
  # name is derived from the bundle's directory, so 'compose-caddy-1' is a
  # coincidence of the current path rather than a contract.
  local found cname
  found="$(ssh_vm 'sudo docker ps --filter label=com.docker.compose.service=caddy --format "{{.Names}}"' || true)"
  cname="${found%%$'\n'*}"

  if [ -z "$cname" ]; then
    warn "no running Caddy container on the VM."
    cat >&2 <<EOF

The 'tls internal' CA is generated by Caddy the first time it starts, so it does
not exist yet. Bring the stack up with Caddy enabled (chunk 07), then:

  ./host-dns.sh --ca-only
EOF
    exit 3
  fi
  info "caddy container: $cname"

  mkdir -p "$CA_DIR"
  local tmp
  tmp="$(mktemp "${TMPDIR:-/tmp}/buzz-caddy-root.XXXXXX")"
  # shellcheck disable=SC2064
  trap "rm -f '$tmp'" EXIT

  if ! ssh_vm "sudo docker exec $cname cat $CADDY_CA_PATH" > "$tmp"; then
    warn "could not read $CADDY_CA_PATH inside $cname."
    warn "if Caddy is running without 'tls internal' there is no local CA to fetch -- see hardening-spec.md §A.3."
    exit 4
  fi

  if ! grep -q 'BEGIN CERTIFICATE' "$tmp"; then
    warn "what came back is not a PEM certificate. Refusing to install it."
    exit 4
  fi

  if [ -f "$CA_OUT" ] && cmp -s "$tmp" "$CA_OUT"; then
    info "unchanged: $CA_OUT"
  else
    local verb="wrote"
    [ -f "$CA_OUT" ] && verb="replaced (the VM's CA changed)"
    mv "$tmp" "$CA_OUT"
    chmod 0644 "$CA_OUT"
    info "$verb: $CA_OUT"
  fi

  if command -v openssl >/dev/null 2>&1; then
    say "Certificate"
    openssl x509 -in "$CA_OUT" -noout -subject -issuer -dates || true
    openssl x509 -in "$CA_OUT" -noout -fingerprint -sha256 || true
  fi

  print_trust_command "$CA_OUT"
}

# ---------------------------------------------------------------------------

[ "$DO_DNS" -eq 1 ] && do_dns
[ "$DO_CA"  -eq 1 ] && do_ca

if [ "$DO_CA" -eq 0 ]; then
  cat <<EOF
    next: https://${DOMAIN}:8443 and https://${ADMIN_DOMAIN}:8443/reports once
    chunk 07 is up. Both warn about the certificate until you run:
      ./host-dns.sh --ca-only
EOF
fi
