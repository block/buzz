#!/usr/bin/env bash
# Chunk 00 -- PREFLIGHT. Host-side checks that must pass before anything else.
#
# Read-only. This script inspects the host and changes nothing, so it is safe to
# run as often as you like. It covers dev-deployment-SOP.md Steps 0.2, 0.3, 0.4,
# 1 and 3.2 -- the "before you start" conditions that, when missing, fail much
# later and look like faults in the VM rather than faults on the host.
#
# The reasoning for every check lives in the SOP step cited beside it. This file
# holds the operation only.
#
# Exit status: 0 if every check passed (warnings are allowed), 1 if any failed.
set -euo pipefail

# ---------------------------------------------------------------------------
# Bash traps this script is written around. Do not undo these.
#
# 1. SIGPIPE / exit 141. With `set -o pipefail`, piping a producer into a
#    short-reading consumer (`... | head -c 32`, `grep ... | head -1`,
#    `awk '{print; exit}'`) SIGPIPEs the producer, and pipefail turns that into
#    a hard failure -- the script dies with 141 before it logs anything. This
#    exact bug was hit in build-vps-clone.sh (see the comments at its lines
#    47-51 and 76). So: NO `head` as a pipe consumer anywhere in this file, and
#    no early-exiting awk. Every awk here reads its input to EOF and prints in
#    END, or uses NR== on a stream short enough to drain.
#
# 2. `set -e` plus a failing check aborts before the summary prints. Every
#    check below is invoked failure-tolerantly and records its own result, so
#    the PASS/FAIL table is always reached. Nothing here may `return 1` into
#    bare `set -e` control flow.
#
# 3. `((n++))` evaluates to 0 on the first increment, which `set -e` reads as
#    failure. Counters use `n=$((n+1))` instead.
#
# 4. macOS /bin/bash is 3.2 (2007). Nothing here uses associative arrays,
#    `mapfile`, `${var,,}` or other bash 4+ syntax.
# ---------------------------------------------------------------------------

VM="${VM:-buzz-dev}"

# Same default, and the same override name, as build-vps-clone.sh line 34.
OVA="${OVA:-$HOME/vm-images/noble.ova}"

# build-vps-clone.sh line 37 reads this literal path with no override, so
# neither does preflight -- an override here would pass a check the build then
# fails on. SOP Step 3.2.
PUBKEY_PATH="$HOME/.ssh/id_ed25519.pub"

MIN_FREE_KIB=$((15 * 1024 * 1024))   # 15 GiB, SOP Step 0.3
MIN_OVA_BYTES=500000000              # 500 MB; a full noble.ova is ~570 MB, SOP Step 1

# Host ports the NAT forwards claim: ssh 2222->22, relay 3000->3000, and — for
# the experimental Caddy profile — http 8080->80 and https 8443->443.
#
# 3000 is the DEFAULT path's relay port and the one most likely to collide: it is
# a common default for local Node/Rails servers. macOS permits a non-root bind
# there, so host and guest ports match and nothing is translated.
#
# 80 and 443 are deliberately NOT checked: we never bind them on the host. macOS
# forbids a non-root process binding below 1024, which is the whole reason the
# experimental profile's host-side forwards are high ports while the guest side
# stays the real one. See build-vps-clone.sh.
HOST_PORTS="2222 3000 8080 8443"

# Names chunk 03 adds to /etc/hosts (SOP Step 10). Warning-only here.
HOSTS_NAMES="buzz-vm.test admin.buzz-vm.test"

ROWS=""
FAILED=0
WARNED=0
PASSED=0

record() { # status  name  detail
  ROWS="${ROWS}$1|$2|$3
"
}

# First remedy line is labelled, continuation lines are aligned under it. awk
# drains stdin and prints as it goes -- no early exit, per trap 1.
say_remedy() {
  printf '%s\n' "$1" | awk '
    NR == 1 { printf "           remedy: %s\n", $0; next }
            { printf "                   %s\n", $0 }'
}

pass() { printf '  [ PASS ] %s -- %s\n' "$1" "$2"; record PASS "$1" "$2"; PASSED=$((PASSED+1)); }
warn() { printf '  [ WARN ] %s -- %s\n' "$1" "$2"; say_remedy "$3"; record WARN "$1" "$2"; WARNED=$((WARNED+1)); }
fail() { printf '  [ FAIL ] %s -- %s\n' "$1" "$2"; say_remedy "$3"; record FAIL "$1" "$2"; FAILED=$((FAILED+1)); }

# File size in bytes. `wc -c` with a redirect (not a pipe) is portable across
# BSD and GNU, unlike stat's incompatible -f%z / -c%s spellings. The arithmetic
# expansion strips the leading whitespace BSD wc pads its output with.
file_size_bytes() {
  printf '%s' "$(( $(wc -c < "$1") ))"
}

# First line of a command's output matching a pattern, without early-exiting the
# reader. Drains stdin, prints in END.
first_matching_line() { # pattern   (stdin = output to scan)
  awk -v pat="$1" '$0 ~ pat && !seen { v = $0; seen = 1 } END { print v }'
}

# Addresses a name resolves to, one per line. Ladder: macOS resolver, glibc
# resolver, then the hosts file itself.
resolve_host() {
  if command -v dscacheutil >/dev/null 2>&1; then
    dscacheutil -q host -a name "$1" 2>/dev/null | awk '/^ip_address:/ { print $2 }'
  elif command -v getent >/dev/null 2>&1; then
    getent hosts "$1" 2>/dev/null | awk '{ print $1 }'
  else
    awk -v n="$1" '$0 !~ /^[[:space:]]*#/ { for (i = 2; i <= NF; i++) if ($i == n) print $1 }' \
      /etc/hosts 2>/dev/null
  fi
}

# First listener on a TCP port as "command (pid N)", empty when the port is free.
# lsof exits 1 when it matches nothing, hence the `|| true`. Note lsof without
# sudo only sees processes you own; a port held by another user's process reads
# as free here.
port_listener() {
  local out
  out="$(lsof -nP -iTCP:"$1" -sTCP:LISTEN 2>/dev/null || true)"
  [ -n "$out" ] || return 0
  printf '%s\n' "$out" | awk 'NR == 2 { printf "%s (pid %s)", $1, $2 }'
}

printf '\n=== Chunk 00 -- PREFLIGHT (%s) ===\n\n' "$(date '+%Y-%m-%d %H:%M:%S')"

# --- 1. Host platform ------------------------------------------------------
# Not an SOP check: the SOP's manual path supports Windows via Git Bash, but
# this automated chain does not. build-vps-clone.sh builds the cloud-init seed
# with `hdiutil makehybrid`, which is macOS-only, so the scripted path requires
# a Mac. Flagged as a failure rather than left to surface as "hdiutil: command
# not found" in chunk 01.
KERNEL="$(uname -s)"
DATA_VOL="/"
if [ "$KERNEL" = "Darwin" ]; then
  DATA_VOL="/System/Volumes/Data"
  pass "host platform" "Darwin ($(uname -r))"
else
  fail "host platform" "uname -s is '$KERNEL', expected Darwin" \
    "The scripted path requires macOS: build-vps-clone.sh builds the cloud-init
seed ISO with 'hdiutil makehybrid', which exists only on macOS.
Follow dev-deployment-SOP.md Steps 1-3 by hand on this platform, or run the
chunks from a Mac."
fi

# --- 2. Processor architecture (SOP Step 0.2 -- HARD STOP) ------------------
ARCH="$(uname -m)"
if [ "$ARCH" = "x86_64" ]; then
  pass "processor architecture" "x86_64"
else
  fail "processor architecture" "uname -m is '$ARCH', expected x86_64 -- HARD STOP" \
    "There is no way round this on this machine. VirtualBox cannot run x86_64
guests on ARM hardware, and the guest architecture has to match the
production VPS or the environment stops being a mirror of it -- capacity
and behaviour measured on an ARM guest do not transfer.
Affects Apple Silicon Macs (M1-M4) and Windows-on-ARM PCs.
Use an Intel Mac or an AMD/Intel PC, or build this on a cloud VM.
See dev-deployment-SOP.md Step 0.2."
fi

# --- 3. Free disk on the host data volume (SOP Step 0.3) -------------------
AVAIL_KIB="$(df -Pk "$DATA_VOL" 2>/dev/null | awk 'NR == 2 { print $4 }' || true)"
if [ -z "$AVAIL_KIB" ]; then
  fail "free disk space" "could not read free space on $DATA_VOL" \
    "Run 'df -Pk $DATA_VOL' by hand and check the Avail column. SOP Step 0.3."
else
  AVAIL_H="$(awk -v k="$AVAIL_KIB" 'BEGIN { printf "%.1f GiB", k / 1048576 }')"
  if [ "$AVAIL_KIB" -ge "$MIN_FREE_KIB" ]; then
    pass "free disk space" "$AVAIL_H free on $DATA_VOL (need 15 GiB)"
  else
    fail "free disk space" "only $AVAIL_H free on $DATA_VOL, need 15 GiB" \
      "Free space before building. The VM's disk file grows as the guest writes
to it, so running the host out of space mid-install produces failures that
look like faults inside the VM. SOP Step 0.3."
  fi
fi

# --- 4. VirtualBox command-line tools (SOP Step 0.4) -----------------------
if command -v VBoxManage >/dev/null 2>&1; then
  VBOX_VER="$(VBoxManage --version 2>/dev/null | first_matching_line '^[0-9]' || true)"
  if [ -n "$VBOX_VER" ]; then
    pass "VBoxManage" "version $VBOX_VER at $(command -v VBoxManage)"
  else
    fail "VBoxManage" "found at $(command -v VBoxManage) but --version printed nothing usable" \
      "The VirtualBox kernel extension is probably not loaded. Open System
Settings > Privacy & Security, allow the Oracle system extension, reboot,
then re-run 'VBoxManage --version'. SOP Step 0.4."
  fi
else
  fail "VBoxManage" "not on PATH" \
    "Install VirtualBox (the x86/amd64 host package) from
https://www.virtualbox.org/wiki/Downloads
On macOS the tool lands in /usr/local/bin/VBoxManage; if it is installed but
not found, add that directory to PATH. SOP Step 0.4."
fi

# --- 5. SSH public key (SOP Step 3.2) --------------------------------------
# Existence and shape only. The key's contents are never printed: AGENTS.md
# rule 2 forbids committing SSH public keys, and a log or terminal capture is
# how they leak into the repo. build-vps-clone.sh reads this file at run time.
if [ -f "$PUBKEY_PATH" ]; then
  PUBKEY_BYTES="$(file_size_bytes "$PUBKEY_PATH")"
  PUBKEY_TYPE="$(awk 'NR == 1 { print $1 }' "$PUBKEY_PATH" 2>/dev/null || true)"
  if [ "$PUBKEY_BYTES" -lt 60 ] || [ "$PUBKEY_TYPE" != "ssh-ed25519" ]; then
    fail "SSH public key" "$PUBKEY_PATH is empty or not an ssh-ed25519 key" \
      "The build script embeds this file verbatim into cloud-init YAML, so a
truncated or wrong-type key produces a VM you cannot log into.
Regenerate: ssh-keygen -t ed25519 -C \"buzz-dev\"
Accept the default path. SOP Step 3.2."
  else
    pass "SSH public key" "$PUBKEY_PATH present (${PUBKEY_BYTES} bytes, ssh-ed25519)"
  fi
else
  fail "SSH public key" "$PUBKEY_PATH not found" \
    "build-vps-clone.sh reads this exact path to authorise you on the guest.
Create one: ssh-keygen -t ed25519 -C \"buzz-dev\"
Press Enter at each prompt to accept the defaults. SOP Step 3.2."
fi

# --- 6. Ubuntu cloud image (SOP Step 1) ------------------------------------
if [ -f "$OVA" ]; then
  OVA_BYTES="$(file_size_bytes "$OVA")"
  OVA_H="$(awk -v b="$OVA_BYTES" 'BEGIN { printf "%.0f MB", b / 1000000 }')"
  if [ "$OVA_BYTES" -ge "$MIN_OVA_BYTES" ]; then
    pass "noble.ova" "$OVA ($OVA_H)"
  else
    fail "noble.ova" "$OVA is only $OVA_H, expected at least 500 MB" \
      "The download was truncated -- a partial OVA imports with a confusing
error much later. Delete and re-fetch:
  rm -f \"$OVA\"
  curl -L -o \"$OVA\" \\
    https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.ova
SOP Step 1."
  fi
else
  fail "noble.ova" "$OVA not found" \
    "Download the Ubuntu 24.04 cloud image in .ova form (~570 MB):
  mkdir -p \"\$(dirname \"$OVA\")\"
  curl -L -o \"$OVA\" \\
    https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.ova
Use .ova, not .img: the .img is QCOW2, which VirtualBox cannot read.
Override the location with OVA=/path/to/noble.ova. SOP Step 1."
fi

# --- 7. Ansible on the control node ---------------------------------------
# Everything that runs on the target is Ansible's, from the first chunk on, so
# the control node needs it before chunk 02 builds anything.
ANSIBLE_MISSING=""
command -v ansible >/dev/null 2>&1 || ANSIBLE_MISSING="ansible"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  ANSIBLE_MISSING="${ANSIBLE_MISSING:+$ANSIBLE_MISSING and }ansible-playbook"
fi
if [ -n "$ANSIBLE_MISSING" ]; then
  fail "ansible" "$ANSIBLE_MISSING not on PATH" \
    "Install Ansible on this machine (the control node):
  brew install ansible
or, without Homebrew:
  python3 -m pip install --user ansible-core
Then reopen the terminal so PATH picks it up."
else
  ANSIBLE_VER="$(ansible --version 2>/dev/null | first_matching_line '^ansible' || true)"
  if [ -n "$ANSIBLE_VER" ]; then
    pass "ansible" "$ANSIBLE_VER"
  else
    fail "ansible" "both commands are on PATH but 'ansible --version' failed" \
      "Run 'ansible --version' by hand and read the error. A broken Python
environment is the usual cause -- reinstall with 'brew reinstall ansible'
or into a clean virtualenv."
  fi
fi

# --- 8. Host ports the NAT forwards need ----------------------------------
if command -v lsof >/dev/null 2>&1; then
  for p in $HOST_PORTS; do
    holder="$(port_listener "$p")"
    if [ -z "$holder" ]; then
      pass "host port $p free" "nothing listening on 127.0.0.1:$p"
    else
      case "$holder" in
        VBox*|VirtualBox*)
          fail "host port $p free" "held by $holder" \
            "That is VirtualBox, so the '$VM' VM is probably already running and
holding its own forward. Chunk 01 starts by deleting and rebuilding the VM,
which fails while it is running. Power it off first:
  VBoxManage controlvm $VM poweroff
Then re-run this preflight. SOP Step 2.7."
          ;;
        *)
          fail "host port $p free" "held by $holder" \
            "Stop that process, or the VM's port forward will not bind and the
guest becomes unreachable on this port. Identify it fully with:
  lsof -nP -iTCP:$p -sTCP:LISTEN
The four host ports are fixed in build-vps-clone.sh (SSH_PORT, RELAY_PORT,
HTTP_PORT, HTTPS_PORT); change them there, not by hand, or a rebuild loses the
change.
SOP Step 2.7."
          ;;
      esac
    fi
  done
else
  warn "host ports $HOST_PORTS" "lsof not available, port checks skipped" \
    "Check by hand that nothing is listening on 2222, 3000, 8080 and 8443 before
running chunk 02. SOP Step 2.7."
fi

# --- 9. Hosts-file entries (SOP Step 10) -- WARNING ONLY -------------------
# Chunk 03 is what adds these, so their absence before that point is expected
# and must not fail preflight.
for h in $HOSTS_NAMES; do
  addrs="$(resolve_host "$h" || true)"
  addr_list=" $(printf '%s' "$addrs" | tr '\n' ' ') "
  case "$addr_list" in
    *" 127.0.0.1 "*)
      pass "$h resolves" "127.0.0.1"
      ;;
    "  ")
      warn "$h resolves" "does not resolve yet" \
        "Expected before chunk 03 -- that chunk adds the entry. Not a failure.
Chunk 03 runs, in effect:
  sudo sh -c 'printf \"\\n127.0.0.1 $h\\n\" >> /etc/hosts'
SOP Step 10."
      ;;
    *)
      warn "$h resolves" "resolves to$addr_list(not 127.0.0.1)" \
        "Something else already claims this name. The browser and the desktop
app must reach the host's port forwards, so it has to be 127.0.0.1.
Fix the /etc/hosts entry, or clear whatever DNS or VPN search domain is
answering for it. SOP Step 10."
      ;;
  esac
done

# --- Summary --------------------------------------------------------------
printf '\n--- PREFLIGHT SUMMARY ---\n\n'
printf '  %-6s  %-32s  %s\n' "RESULT" "CHECK" "DETAIL"
printf '  %-6s  %-32s  %s\n' "------" "--------------------------------" "------"
printf '%s' "$ROWS" | awk -F'|' 'NF { printf "  %-6s  %-32s  %s\n", $1, $2, $3 }'
printf '\n  %d passed, %d warning(s), %d failed\n\n' "$PASSED" "$WARNED" "$FAILED"

if [ "$FAILED" -gt 0 ]; then
  printf 'PREFLIGHT FAILED. Fix the remedies printed above and run this again.\n'
  printf 'Do not start chunk 02 -- every failure here surfaces later as something\n'
  printf 'that looks like a fault in the VM.\n\n'
  exit 1
fi

printf 'PREFLIGHT PASSED. Next: chunk 01 (fetch the OVA), then chunk 02 (build the VM).\n'
if [ "$WARNED" -gt 0 ]; then
  printf 'Warnings above are informational and do not block chunk 01 or 02.\n'
fi
printf '\n'
