#!/usr/bin/env bash
# Build a local VirtualBox VM matching the production VPS.
#
# VPS being mirrored:  Ubuntu 24.04 LTS (noble), x86_64, 1 vCPU, 1.9Gi RAM,
#                      49.5G root disk, 496M swap.
# Local deviation:     root disk is 10G (host has limited free space).
set -euo pipefail

VM="${VM:-buzz-dev}"
CPUS=1
RAM_MB=2048          # matches the VPS's 1.9Gi
DISK_MB=20480        # 20G -- deliberately smaller than the VPS's 49.5G
SSH_PORT=2222
SWAP_BYTES=520093696 # 496 MiB, matching the VPS swap partition

# Caddy fronts the relay on 80/443 inside the guest (compose.caddy.yml), and
# `ports: !reset []` means the relay's own 3000 is never published. macOS will
# not let a non-root process bind a port below 1024, so the host side of each
# forward is a high port. The guest side is the real one, so the container
# configuration is byte-identical to production's.
HTTP_PORT=8080       # host 8080 -> guest 80
HTTPS_PORT=8443      # host 8443 -> guest 443

# Guest hostname and the non-root fallback account. These names are load-bearing
# downstream: the hardening role removes the sshd parity file by name, and
# hardening-spec.md Part D asserts on it. Keep all three in step.
GUEST_HOSTNAME="${GUEST_HOSTNAME:-buzz-dev}"
FALLBACK_USER="${FALLBACK_USER:-dev}"
PARITY_CONF="01-dev-parity.conf"

# Where the Ubuntu cloud image lives, and a scratch area for the cloud-init seed.
# Both overridable. Get the OVA from:
#   https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.ova
OVA="${OVA:-$HOME/vm-images/noble.ova}"
SCRATCH="${SCRATCH:-$HOME/vm-images/.build-$VM}"
VMDIR="$HOME/VirtualBox VMs/$VM"
PUBKEY="$(cat "$HOME/.ssh/id_ed25519.pub")"

# Console-fallback password for root and the fallback user, used only from the
# VirtualBox console window when the guest's network is broken. Deliberately NOT
# committed: a hash in version control is a shared password on a root account,
# and #17's definition of done forbids it.
#
# Supply your own with `PWHASH="$(openssl passwd -6)" ./build-vps-clone.sh`, or
# let this generate a random one and drop it in a gitignored file next door.
CREDS="${CREDS:-$HOME/vm-images/.${VM}-console-password}"
if [ -z "${PWHASH:-}" ]; then
  # No pipe into `head` here: with `set -o pipefail` a short-reading `head`
  # SIGPIPEs its producer and kills the script with 141 before it logs a thing.
  _pw="$(openssl rand -hex 12)"
  PWHASH="$(printf '%s' "$_pw" | openssl passwd -6 -stdin)"
  ( umask 077; printf 'vm: %s\nuser: root and %s\nconsole password: %s\n' \
      "$VM" "$FALLBACK_USER" "$_pw" > "$CREDS" )
  unset _pw
  echo "generated a random console password -> $CREDS (mode 600, not in the repo)"
fi

[ -f "$OVA" ] || { echo "OVA not found: $OVA (set OVA=/path/to/noble.ova)"; exit 1; }

say() { printf '\n=== %s ===\n' "$1"; }

say "Cleaning any previous build"
VBoxManage unregistervm "$VM" --delete 2>/dev/null || true
rm -rf "$VMDIR"

say "Importing OVA as $VM"
VBoxManage import "$OVA" --vsys 0 --vmname "$VM" --basefolder "$HOME/VirtualBox VMs"

# The OVA ships a VMDK. modifymedium --resize only works on VDI/VHD, so the
# disk has to be converted or the "resize it later" workflow silently fails.
say "Locating imported disk"
# The OVA attaches its disk to a SCSI controller, which is NOT controller 0
# (that's the IDE one). Parse the actual attachment line -- "SCSI-0-0"="/path.vmdk"
# -- so the controller name and port/device always come from reality.
# Take the first match with a parameter expansion rather than `| head -1`: head
# exiting early SIGPIPEs grep, which `set -o pipefail` turns into a hard failure.
ATTACH="$(VBoxManage showvminfo "$VM" --machinereadable | grep -E '^"[^"]+-[0-9]+-[0-9]+"="[^"]*\.vmdk"$' || true)"
ATTACH="${ATTACH%%$'\n'*}"
SLOT="${ATTACH%%=*}"; SLOT="${SLOT//\"/}"       # e.g. SCSI-0-0
VMDK="${ATTACH#*=}"; VMDK="${VMDK//\"/}"
DEV="${SLOT##*-}"; REST="${SLOT%-*}"
PORT="${REST##*-}"; CTL="${REST%-*}"
[ -n "$VMDK" ] && [ -n "$CTL" ] || { echo "could not locate imported disk"; exit 1; }
echo "disk:       $VMDK"
echo "controller: $CTL (port $PORT, device $DEV)"
VDI="$VMDIR/$VM.vdi"

say "Converting VMDK -> VDI (required for resize support)"
VBoxManage clonemedium disk "$VMDK" "$VDI" --format VDI

say "Swapping the VDI in and reclaiming the VMDK"
VBoxManage storageattach "$VM" --storagectl "$CTL" --port "$PORT" --device "$DEV" --medium none
VBoxManage closemedium disk "$VMDK" --delete
VBoxManage storageattach "$VM" --storagectl "$CTL" --port "$PORT" --device "$DEV" \
  --type hdd --medium "$VDI"

say "Resizing disk to ${DISK_MB}MB"
VBoxManage modifymedium disk "$VDI" --resize "$DISK_MB"

say "Building cloud-init seed ISO"
SEED="$SCRATCH/seed"
rm -rf "$SEED"; mkdir -p "$SEED"

cat > "$SEED/meta-data" <<EOF
instance-id: ${VM}-001
local-hostname: ${GUEST_HOSTNAME}
EOF

cat > "$SEED/user-data" <<EOF
#cloud-config
hostname: ${GUEST_HOSTNAME}
manage_etc_hosts: true

# The VPS is accessed as 'ssh root@<ip>', so root login is enabled here to match.
# '${FALLBACK_USER}' is kept only as a fallback so a bad sshd edit can't lock you out.
disable_root: false

users:
  - default
  - name: root
    ssh_authorized_keys:
      - ${PUBKEY}
  - name: ${FALLBACK_USER}
    gecos: Dev
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    lock_passwd: false
    passwd: "${PWHASH}"
    ssh_authorized_keys:
      - ${PUBKEY}

ssh_pwauth: true

chpasswd:
  expire: false
  users:
    - name: root
      password: "${PWHASH}"
      type: hash
    - name: ${FALLBACK_USER}
      password: "${PWHASH}"
      type: hash

write_files:
  # sshd uses the FIRST value it obtains for a keyword, and the cloud image's
  # own 60-cloudimg-settings.conf sets 'PermitRootLogin prohibit-password'.
  # This file must sort BEFORE that one to win, hence the 01- prefix.
  #
  # The hardening role REMOVES this file by name. If you rename it here, rename
  # it there too -- adding a hardening file alongside this one changes nothing,
  # because sshd keeps the first value it read. See hardening-spec.md C2.
  - path: /etc/ssh/sshd_config.d/${PARITY_CONF}
    permissions: '0644'
    content: |
      PermitRootLogin yes
      PasswordAuthentication yes

runcmd:
  - [ systemctl, restart, ssh ]

# The cloud image ships no swap; the VPS has a 496M swap partition. Match it so
# memory-pressure behaviour at 2GB RAM is comparable.
swap:
  filename: /swapfile
  size: ${SWAP_BYTES}

package_update: false
package_upgrade: false
EOF

# macOS has no genisoimage/cloud-localds. hdiutil can produce the ISO, but the
# volume label MUST be exactly "cidata" or cloud-init's NoCloud source ignores it.
rm -f "$SCRATCH/seed.iso"
hdiutil makehybrid -iso -joliet -default-volume-name cidata \
  -o "$SCRATCH/seed.iso" "$SEED" >/dev/null
echo "seed.iso built:"
hdiutil imageinfo "$SCRATCH/seed.iso" | grep -iA1 "Volume Name" | head -4 || true

say "Attaching seed ISO"
VBoxManage storagectl "$VM" --name "IDE" --add ide --controller PIIX4 2>/dev/null || true
VBoxManage storageattach "$VM" --storagectl "IDE" --port 0 --device 0 \
  --type dvddrive --medium "$SCRATCH/seed.iso"

say "Setting CPU / RAM / network to match VPS"
VBoxManage modifyvm "$VM" \
  --cpus "$CPUS" \
  --memory "$RAM_MB" \
  --nic1 nat \
  --audio-driver none \
  --graphicscontroller vmsvga \
  --vram 16
say "Adding NAT port forwards"
# Bound to 127.0.0.1 so nothing off this machine can reach the guest -- that is
# what stands in for "no public listener" on the VPS (hardening-spec.md C1).
for rule in "ssh,tcp,127.0.0.1,${SSH_PORT},,22" \
            "http,tcp,127.0.0.1,${HTTP_PORT},,80" \
            "https,tcp,127.0.0.1,${HTTPS_PORT},,443"; do
  name="${rule%%,*}"
  VBoxManage modifyvm "$VM" --natpf1 delete "$name" 2>/dev/null || true
  VBoxManage modifyvm "$VM" --natpf1 "$rule"
done
VBoxManage showvminfo "$VM" --machinereadable | grep -i '^Forwarding'

say "Starting headless"
VBoxManage startvm "$VM" --type headless

echo
echo "VM '$VM' is booting. cloud-init takes ~60-90s on first boot."
echo "SSH:   ssh -p ${SSH_PORT} root@127.0.0.1                 # matches the VPS access pattern"
echo "       ssh -p ${SSH_PORT} ${FALLBACK_USER}@127.0.0.1                  # fallback account"
echo "Relay: https://buzz-vm.test:${HTTPS_PORT}         # once the stack is up (chunk 07)"
echo
echo "Note: the guest hostname is '${GUEST_HOSTNAME}'; the relay's community host is"
echo "'buzz-vm.test:${HTTPS_PORT}'. They are deliberately different things -- the community"
echo "host comes from RELAY_URL alone. See runbooks/relay-build-list.md."
