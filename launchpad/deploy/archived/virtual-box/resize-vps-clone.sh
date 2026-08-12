#!/usr/bin/env bash
# Resize the vps-clone-noble VM. Disk grows only; VirtualBox cannot shrink a VDI.
#
#   ./resize-vps-clone.sh disk 20480     # grow root disk to 20 GB
#   ./resize-vps-clone.sh ram  4096      # 4 GB RAM
#   ./resize-vps-clone.sh cpus 2         # 2 vCPUs
set -euo pipefail

VM="vps-clone-noble"
VDI="$HOME/VirtualBox VMs/$VM/$VM.vdi"
WHAT="${1:-}"; VALUE="${2:-}"
[ -n "$WHAT" ] && [ -n "$VALUE" ] || { sed -n '2,8p' "$0"; exit 1; }

running() { VBoxManage list runningvms | grep -q "\"$VM\""; }

if running; then
  echo "Powering off $VM..."
  VBoxManage controlvm "$VM" acpipowerbutton
  for _ in $(seq 1 30); do running || break; sleep 2; done
  running && { VBoxManage controlvm "$VM" poweroff; sleep 3; }
fi

case "$WHAT" in
  disk)
    CUR=$(VBoxManage showmediuminfo disk "$VDI" | awk '/^Capacity:/{print $2}')
    [ "$VALUE" -gt "$CUR" ] || { echo "ERROR: can only grow (current ${CUR}MB, asked ${VALUE}MB)"; exit 1; }
    echo "Free space on host before growing:"; df -h /System/Volumes/Data | tail -1
    VBoxManage modifymedium disk "$VDI" --resize "$VALUE"
    NEEDS_GROW=1
    ;;
  ram)  VBoxManage modifyvm "$VM" --memory "$VALUE" ;;
  cpus) VBoxManage modifyvm "$VM" --cpus "$VALUE" ;;
  *)    echo "unknown target '$WHAT' (use: disk | ram | cpus)"; exit 1 ;;
esac

echo "Starting $VM..."
VBoxManage startvm "$VM" --type headless

if [ "${NEEDS_GROW:-0}" = "1" ]; then
  echo "Waiting for SSH, then expanding the partition inside the guest..."
  for _ in $(seq 1 40); do
    ssh -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=8 -o BatchMode=yes root@127.0.0.1 true 2>/dev/null && break
    sleep 5
  done
  # Enlarging the VDI only gives the guest a bigger raw device; the partition and
  # filesystem still have to catch up. In practice cloud-init's growpart module
  # runs on every boot and has already done it by the time we get here (these
  # commands then report NOCHANGE / "Nothing to do", which is success, not error).
  # They are kept as a fallback for images where growpart is disabled.
  ssh -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o BatchMode=yes root@127.0.0.1 \
      'growpart /dev/sda 1 || true; resize2fs /dev/sda1; echo; df -h /' 2>&1 | grep -v "Permanently added"
fi

echo "Done."
