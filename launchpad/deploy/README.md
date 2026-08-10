# vps-clone-noble — local VirtualBox mirror of the buzz-server VPS

A local VM that mirrors the production VPS closely enough to test whether
software actually fits, without paying for a bigger plan to find out.

Staging location for `launchpad/dev/deploy/`; migration into the repo is pending.
Read [AGENTS.md](./AGENTS.md) first — it carries the mirror/configure boundary and
the rules that keep the VM's measurements trustworthy.

## Layout

| Path | What |
|---|---|
| `virtual-box/` | Build and resize scripts, cloud-init. Mirrors the VPS's starting state. Dev-only |
| `ansible/` | Provisioning applied to that state. Runs against **both** the VM and the VPS |
| `scripts/` | Target-agnostic helpers, runnable on either host |
| `docker/` | Containerised supporting tools. Empty on purpose |
| `runbooks/` | Committed procedures — start with `runbooks/relay-build-list.md` |

## Access

```bash
ssh -p 2222 root@127.0.0.1      # matches the VPS's `ssh root@<ip>` pattern
ssh -p 2222 jeff@127.0.0.1      # fallback account (passwordless sudo)
```

Auth is via `~/.ssh/id_ed25519`. Password for both accounts is `vpsclone`
(console fallback). Port 2222 is bound to `127.0.0.1` only — not reachable
from your LAN.

## Parity with the VPS

| | VPS | Local clone | |
|---|---|---|---|
| OS | Ubuntu 24.04.4 LTS (noble) | Ubuntu 24.04.4 LTS (noble) | match |
| Arch | x86_64 | x86_64 | match |
| Kernel | 6.8.0-137-generic | 6.8.0-136-generic | one point release behind |
| vCPUs | 1 | 1 | match |
| RAM | 1.9 Gi | 1.9 Gi | match |
| Swap | 496 M (partition) | 496 M (file) | same size, different backing |
| Root disk | 49.5 G (44 G free) | 12 G (8.5 G free) | **deliberately smaller** |
| Root login | `ssh root@` | `ssh root@` | match |
| Docker | not installed | installed by Ansible | deliberate — the VPS gets the same role |

### Differences that could bite

- **Disk is ~4x smaller.** Testing "does it fit" against 8.5 G free proves
  nothing about 44 G free. It is good for catching software that is *too big*,
  useless for confirming something *will* fit. Grow it before drawing
  conclusions about capacity.
- **Partition layout differs.** The VPS has a bare `ext4` filesystem written
  straight to `/dev/sda` with no partition table. The cloud image uses a
  partition table (`sda1` root, plus `/boot` and `/boot/efi`). Irrelevant for
  app testing; relevant if you script anything touching partitions.
- **Swap is a file, not a partition.** Same size and priority; behaves the same
  under memory pressure.
- **Disk I/O is not comparable.** Host is a laptop SSD behind VirtualBox;
  the VPS is QEMU virtio on the provider's storage.

## Resizing

Disks can only grow — VirtualBox cannot shrink a VDI.

```bash
./virtual-box/resize-vps-clone.sh disk 20480    # root disk -> 20 GB
./virtual-box/resize-vps-clone.sh ram  4096     # RAM -> 4 GB
./virtual-box/resize-vps-clone.sh cpus 2        # 2 vCPUs
```

Changing RAM or vCPU away from 1/2048 destroys parity with the VPS and invalidates
any capacity measurement taken afterwards. Put it back before running #18.

The script powers the VM down, applies the change, reboots, and (for `disk`)
expands the partition and filesystem inside the guest. `NOCHANGE` /
`Nothing to do` in the output means cloud-init's growpart already handled it —
that is success.

**Watch host free space.** The VDI is dynamically allocated: 12 G logical but
only ~1.8 G actually on disk. It grows as the guest writes. Your host had
~6.6 G free at build time, so a guest that genuinely fills 12 G will run the
host out of space first. Check with `df -h /System/Volumes/Data`.

## Rebuilding from scratch

```bash
./virtual-box/build-vps-clone.sh
```

Destroys and recreates the VM. Requires `noble.ova` (~566 MB) from
<https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.ova>
— edit the `OVA` path at the top of the script to point at wherever you keep it.
Change `DISK_MB` there to build at a different size.

**`OVA` currently points at a temp scratchpad from an earlier session.** The file is
still there, but that directory is subject to cleanup and a rebuild will fail once it
goes. Move the OVA somewhere permanent and repoint before relying on this.

Cloud-init is generated **inside the script** from an inline heredoc (~line 60) and
written to a scratch directory. To change provisioning, edit that heredoc.
`virtual-box/seed.sample/` is a stale copy of a previous build's output — nothing
reads it, and it contains a real SSH public key, so it is gitignored. An earlier
version of this README said to edit those files; that was wrong.

## What this VM will need to run buzz

**Superseded by [`runbooks/relay-build-list.md`](./runbooks/relay-build-list.md)**, which is
verified against the source rather than recorded from the docs. Read that first. Two
corrections it makes to what follows:

- **`RELAY_URL` is the variable that decides whether anything can connect.** There is no
  command to seed the `communities` table; the relay does it at startup from this one
  variable, and a mismatch rejects every connection while the stack looks healthy.
- The Compose ≥ 2.24.4 requirement is real, but Ubuntu 24.04's archive version sits close
  to that line rather than being certainly too old. Use Docker's own apt repo for a known
  version.

Original notes, kept for context:

Use **`deploy/compose/`**, not the root `docker-compose.yml`. The repo says this
explicitly: the root compose is local dev infrastructure (it adds Keycloak,
Adminer and Prometheus), while `deploy/compose/` is the single-node/VPS bundle.

VPS stack: `relay` + `postgres:17-alpine` + `redis:7-alpine` + `minio` +
a `minio-init` one-shot. Optional `caddy:2-alpine` override for Let's Encrypt TLS.

### Blockers / things to check before trusting a fit test

- **Docker is not installed on the VPS.** The whole deployment is Compose-based
  and needs **Compose v2.24.4+** — the TLS override uses the `!reset` tag, which
  older versions do not parse. Install this on the clone first to make the test
  meaningful.
- **RAM is the real risk.** The root dev compose declares 1600 MB of limits
  (512 Postgres + 512 Keycloak + 256 MinIO + 128 Redis + 128 Prometheus +
  64 Adminer) — already over budget on 1.9 Gi before the relay starts. The VPS
  bundle is leaner and declares *no* limits at all, so nothing stops it
  overcommitting. Swap is only 496 M.
- **Do not build from source.** `BUZZ_IMAGE` defaults to the prebuilt
  `ghcr.io/block/buzz:main`. Compiling this Rust workspace (~30 crates, 270 KB
  `Cargo.lock`) on 1 vCPU / 2 GB would thrash or OOM. Pin to
  `ghcr.io/block/buzz:sha-<7>` for anything reproducible.
- **`BUZZ_AUTO_MIGRATE` is opt-in**, and off by default. A fresh database needs
  it set `true`, or `buzz-admin migrate` run before first start.

### Ports to forward when testing

Only `2222 -> 22` exists today. Add as needed:

| Port | What |
|---|---|
| 3000 | relay HTTP (`BUZZ_HTTP_PORT`) |
| 8080 | health — `/_liveness`, `/_readiness` (container-internal) |
| 9102 | Prometheus metrics (container-internal) |
| 80 / 443 | only with the Caddy TLS override |

```bash
VBoxManage modifyvm vps-clone-noble --natpf1 "buzz,tcp,127.0.0.1,3000,,3000"
```

Smoke test from the repo's own validation steps:
`curl -fsS http://127.0.0.1:3000/_liveness`

## Notes on how this was built

- Ubuntu's `.img` cloud image is QCOW2 and VirtualBox cannot read it; the `.ova`
  imports natively, which is why it's used here.
- The OVA ships a VMDK, but `VBoxManage modifymedium --resize` only works on
  VDI/VHD — hence the VMDK→VDI conversion step. Skipping it makes resizing
  silently impossible.
- The cloud-init seed ISO's volume label must be exactly `cidata` or the guest
  boots with no usable login.
- `PermitRootLogin` is set in `/etc/ssh/sshd_config.d/01-vps-parity.conf`. The
  `01-` prefix matters: sshd takes the *first* value it sees, and the cloud
  image's own `60-cloudimg-settings.conf` would otherwise win.
