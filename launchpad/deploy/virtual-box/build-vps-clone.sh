#!/usr/bin/env bash
# Build a local VirtualBox VM matching the production VPS.
#
# VPS being mirrored:  Ubuntu 24.04 LTS (noble), x86_64, 1 vCPU, 1.9Gi RAM,
#                      49.5G root disk, 496M swap.
# Local deviation:     root disk is 10G (host has limited free space).
set -euo pipefail

VM="vps-clone-noble"
CPUS=1
RAM_MB=2048          # matches the VPS's 1.9Gi
DISK_MB=10240        # 10G -- deliberately smaller than the VPS's 49.5G
SSH_PORT=2222
SWAP_BYTES=520093696 # 496 MiB, matching the VPS swap partition

# Where the Ubuntu cloud image lives, and a scratch area for the cloud-init seed.
# Both overridable. Get the OVA from:
#   https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.ova
OVA="${OVA:-$HOME/vm-images/noble.ova}"
SCRATCH="${SCRATCH:-$HOME/vm-images/.build-$VM}"
VMDIR="$HOME/VirtualBox VMs/$VM"
PUBKEY="$(cat "$HOME/.ssh/id_ed25519.pub")"

# Console-fallback password hash for root and the fallback user. Deliberately NOT
# committed: a hash in version control is a shared password on a root account,
# and #17's definition of done forbids it. Generate your own:
#   PWHASH="$(openssl passwd -6)" ./build-vps-clone.sh
PWHASH="${PWHASH:?set PWHASH to a hash from: openssl passwd -6}"

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
ATTACH="$(VBoxManage showvminfo "$VM" --machinereadable | grep -E '^"[^"]+-[0-9]+-[0-9]+"="[^"]*\.vmdk"$' | head -1)"
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
local-hostname: vps-clone
EOF

cat > "$SEED/user-data" <<EOF
#cloud-config
hostname: vps-clone
manage_etc_hosts: true

# The VPS is accessed as 'ssh root@<ip>', so root login is enabled here to match.
# 'jeff' is kept only as a fallback so a bad sshd edit can't lock you out.
disable_root: false

users:
  - default
  - name: root
    ssh_authorized_keys:
      - ${PUBKEY}
  - name: jeff
    gecos: Jeff
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
    - name: jeff
      password: "${PWHASH}"
      type: hash

write_files:
  # sshd uses the FIRST value it obtains for a keyword, and the cloud image's
  # own 60-cloudimg-settings.conf sets 'PermitRootLogin prohibit-password'.
  # This file must sort BEFORE that one to win, hence the 01- prefix.
  - path: /etc/ssh/sshd_config.d/01-vps-parity.conf
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
VBoxManage modifyvm "$VM" --natpf1 delete "ssh" 2>/dev/null || true
VBoxManage modifyvm "$VM" --natpf1 "ssh,tcp,127.0.0.1,${SSH_PORT},,22"

say "Starting headless"
VBoxManage startvm "$VM" --type headless

echo
echo "VM '$VM' is booting. cloud-init takes ~60-90s on first boot."
echo "SSH:  ssh -p ${SSH_PORT} root@127.0.0.1     # matches the VPS access pattern"
echo "      ssh -p ${SSH_PORT} jeff@127.0.0.1     # fallback account"
