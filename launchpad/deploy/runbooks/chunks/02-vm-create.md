# Chunk 02 — vm-create

**What it does.** **Destroys and rebuilds** the `buzz-dev` VM: imports the OVA, converts the disk to
VDI so it can be resized, sets CPU/RAM/swap to match the production VPS, adds the four NAT port
forwards, builds a cloud-init seed ISO, and boots headless.

**SOP steps covered:** 2, 3, 4. Rationale lives there, not here.

> **This chunk is destructive.** It runs `VBoxManage unregistervm --delete` on `buzz-dev` first.
> `./deploy run all` deliberately skips it for that reason.

## Preconditions

- Chunks 00 and 01 pass — `~/vm-images/noble.ova` present, ~15 GB free, VirtualBox installed,
  `~/.ssh/id_ed25519.pub` present (the script installs it into the guest).
- Nothing you care about in the existing `buzz-dev` VM or its snapshots.

## Run

```bash
./deploy run 02
```

Supply your own console-fallback password instead of a generated one:

```bash
PWHASH="$(openssl passwd -6)" ./virtual-box/build-vps-clone.sh
```

Build under a different name, leaving `buzz-dev` alone:

```bash
VM=buzz-dev-scratch ./virtual-box/build-vps-clone.sh
```

## Verify

```bash
# Parity with the VPS: CPU, RAM, swap. NOT disk -- see Traps.
ssh -p 2222 dev@127.0.0.1 'free -h; swapon --show; df -h /; uname -m'
```

Expect ~1.9 Gi memory, 496 M swap, `x86_64`. Measured on 2026-08-12: 1968 MB / 495 MB / 19 G / x86_64.

```bash
# All four forwards must be bound to 127.0.0.1 only.
VBoxManage showvminfo buzz-dev --machinereadable | grep -i '^Forwarding'
# expect: ssh 2222->22, relay 3000->3000,
#         http 8080->80 and https 8443->443 (experimental TLS profile)

# The deliberately-insecure starting state -- this is the test fixture, not a bug.
ssh -p 2222 dev@127.0.0.1 'ls /etc/ssh/sshd_config.d/; sudo sshd -T | grep permitrootlogin'
# expect: 01-dev-parity.conf, 50-cloud-init.conf, 60-cloudimg-settings.conf
#         permitrootlogin yes
```

Then snapshot immediately, before anything is installed:

```bash
./deploy snapshot pristine
```

## Rollback

There is nothing to roll back to — this chunk *is* the rollback. Re-run it for a clean machine.

## Traps

- **The console password is generated and written to `~/vm-images/.buzz-dev-console-password`**
  (mode 0600, outside the repo). An earlier version of this script hardcoded a password hash, which
  breaches issue #17's own definition of done — a hash in version control is a shared password on a
  root account.
- **`set -o pipefail` plus `| head -c N` is exit 141.** Piping `/dev/urandom` into a short-reading
  `head` SIGPIPEs the producer and kills the script *before it logs anything*, which reads as a
  mysterious silent failure. Hit for real on 2026-08-12; the script now uses `openssl rand` with no
  pipe, and takes the first grep match with a parameter expansion rather than `| head -1`.
- **The disk is converted VMDK → VDI on purpose.** `VBoxManage modifymedium --resize` works only on
  VDI and VHD. Skip the conversion and the resize fails *silently*, leaving you wondering why the disk
  never grew.
- **The controller name is read from VirtualBox, not assumed.** The OVA attaches its disk to a SCSI
  controller, which is not controller 0 — that is the IDE one.
- **Disk is deliberately NOT at parity.** 20 GB here versus the VPS's 49.5 GB, to save host space. This
  VM can prove software is *too big* to fit; it can never prove it *will* fit. Do not draw
  storage conclusions from it (SOP Step 2.5).
- **The `relay` forward (3000 -> 3000) serves the DEFAULT path; 8080/8443 serve the experimental
  Caddy + `tls internal` profile.** Both sets are created unconditionally — a forward to a port
  nothing listens on costs nothing, and having both means switching profiles never needs a rebuild. A
  VM built before the `relay` rule existed can take it live in place:
  `VBoxManage controlvm buzz-dev natpf1 "relay,tcp,127.0.0.1,3000,,3000"`.
- **The experimental profile's host ports are high (8080/8443) but its guest ports are real
  (80/443).** macOS refuses a non-root bind below 1024, and VirtualBox host-only networking is
  unavailable on this machine (`/dev/vboxnetctl` missing — the network kext is not loaded). Container
  configuration is therefore identical to production; only the host-side forward differs, and it
  surfaces as the `:8443` in that profile's community host. The default path needs no such
  translation: 3000 is above 1024, so host and guest ports match.
- **The guest hostname (`buzz-dev`) and the community host (`buzz-vm.test:3000`) are different
  things.** The community comes from `RELAY_URL` alone and nothing else
  ([`../relay-build-list.md`](../relay-build-list.md)).
- **Every run of this chunk invalidates `~/.ssh/known_hosts`.** The rebuilt VM generates new SSH host
  keys, so the stale `[127.0.0.1]:2222` entries make ssh abort with `WARNING: REMOTE HOST
  IDENTIFICATION HAS CHANGED!` and **refuse** to connect — not prompt. Hit for real on 2026-08-12,
  where the deleted `vps-clone-noble` had used the same port and left three entries behind. Clear
  them, then reconnect and accept the new key:
  ```bash
  ssh-keygen -R '[127.0.0.1]:2222'
  # or, to accept it without a prompt:
  ssh-keyscan -p 2222 -t ed25519 127.0.0.1 >> ~/.ssh/known_hosts
  ```
- **The cloud-init volume label must be exactly `cidata`.** Get it wrong and the VM boots with no
  users, no SSH key, and no way in. If you cannot log in, suspect this first.
