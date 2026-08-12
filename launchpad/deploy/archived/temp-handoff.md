# Handoff — dev server and PRD #2

Written 2026-08-11. Temporary; fold the useful parts into the permanent docs and delete.

## One paragraph

The Buzz relay now runs on a local VM whose CPU, RAM and swap match the cohort VPS, and the whole
path from bare Ubuntu to a working relay with browser front-ends has been executed by hand and
written up as [`runbooks/dev-deployment-SOP.md`](./runbooks/dev-deployment-SOP.md). **It fits
comfortably** — peak 563 MB against 1.9 GiB, swap never touched. No Ansible has been written yet;
the manual pass was done first deliberately so the roles encode something observed to work. Nothing
has been posted to GitHub, and four drafted issue artifacts are waiting.

## Live state right now

| Thing | State |
|---|---|
| VM `vps-clone-noble` | **Running**, stack up and healthy |
| Access | `ssh -p 2222 root@127.0.0.1` |
| Relay from the host | `http://127.0.0.1:3000` — NAT forward added at runtime |
| Snapshot `pristine-no-docker` | Bare noble, cloud-init done, Docker not installed |
| Snapshot `docker-clean` | **Does not exist** — the SOP tells readers to take one; this VM predates that step |
| Deployment files | `/opt/buzz/compose/` in the VM |
| Front-end bundles | `/opt/buzz/web/`, `/opt/buzz/admin-web/` |
| Owner secret key | `/root/owner-key.txt` in the VM, mode 600. **Throwaway dev key** |

The NAT forward for 3000 was added with `controlvm` while the VM was running, so it **will not
survive** a rebuild from `build-vps-clone.sh` — that script still only creates the 2222 forward.

### Configuration in use

```
BUZZ_IMAGE=ghcr.io/block/buzz:sha-96ae141
RELAY_URL=ws://buzz-vm.test:3000
BUZZ_REQUIRE_RELAY_MEMBERSHIP=true
BUZZ_AUTO_MIGRATE=true
BUZZ_WEB_DIR=/srv/buzz/web
BUZZ_ADMIN_HOST=admin.buzz-vm.test:3000
BUZZ_ADMIN_WEB_DIR=/srv/buzz/admin-web
BUZZ_SERVE_GIT_WEB_GUI=true
```

Restart with the override, **not** `run.sh` — see the decisions below:

```bash
cd /opt/buzz/compose
docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait
```

`buzz-vm.test` and `admin.buzz-vm.test` are **not** in the host's `/etc/hosts` yet, so browser
access needs those two lines adding (Step 10 of the SOP). `curl` with an explicit `Host` header
works without them.

## Measurements — the answer to #18's question

On 1 vCPU / 1.9 GiB / 496 MB swap, image `sha-96ae141`:

| Stage | Mem used | Peak | Swap | Disk |
|---|---|---|---|---|
| Bare VM, no Docker | 296 MB | — | 0 | 2.2 G |
| Docker installed, idle | 449 MB | — | 0 | 2.8 G |
| `docker pull` ×5 | 469 MB | 474 MB | **0** | 3.9 G |
| Migration + first start | 560 MB | **563 MB** | **0** | 3.9 G |
| Steady idle | 573 MB | — | **0** | 3.9 G |

All-healthy in **18 seconds**. Per container at idle: relay **9 MB**, postgres 51 MB, redis 3 MB,
minio 92 MB. Host overhead `dockerd` 78 MB + `containerd` 47 MB. **~1.39 GiB headroom.** Nothing
OOM-killed, nothing restarted.

This is *not* #18's report — it was a manual learning pass, and the VM's disk is far smaller than
the VPS's so it settles nothing about disk. But the memory verdict looks comfortable, and the
remaining risk is concurrency, not idle footprint.

## PRD #2 and its children

| Issue | Title | State |
|---|---|---|
| **#2** | prd 0 — deploy relay to VPS, verify two people can log in | Open. No success criterion met yet: nothing is on the VPS |
| #17 | spec-matched local VM harness | Open. Harness exists but is uncommitted and has defects (below). **Revised body drafted, unposted** |
| #18 | measure whether the stack fits 1 vCPU / 1.9 Gi | Open. Effectively answered by the numbers above, but not as a clean measured run with a pristine start |
| #19 | rehearse Host→community binding and membership gating | Open. **Host binding fully proven.** Membership gating **not** tested. **Corrected body drafted, unposted** |
| #21 | adr: VPS specification | Open, `needs-decision`. Now has real evidence to decide on |
| #22 | deploy to the VPS with real DNS and TLS | Open, not started. Needs a domain and the VPS |
| #23 | verify two people on two machines | Open, not started. Needs #22 and a second human |
| #39 | load-generation tool, capacity under load | Open, not started. The one genuine unknown left |
| #20 | hardening resource cost | Open. Sits under **#5**, not #2, but feeds #21 |

Milestone **M0 was due 2026-08-14** and has passed. Re-baseline it.

### Recommended order from here

1. **#21** — decide the sizing ADR. Evidence now exists; the only gap is load (#39).
2. **#22** — the long pole, and it is blocked on things money and other people control: a domain,
   DNS, the VPS itself. Start those regardless of code.
3. **#23** — needs a second person scheduled. Scheduling, not engineering.
4. **#19** — close the membership-gating gap.
5. **#39** — only genuinely needed if the sizing decision turns out to be marginal.

## What was decided

| Decision | Choice | Why |
|---|---|---|
| Provisioning boundary | `virtual-box/` mirrors the provider's starting state; **Ansible** owns every change to it | One definition serves the VM and the VPS; the VM becomes the rehearsal ground for lockout-risky plays (#29, #30) |
| Docker source | **Ubuntu 24.04 archive** (`docker.io` 29.1.3, `docker-compose-v2` **2.40.3**, containerd 2.2.1) | Archive Compose is well above the 2.24.4 floor, so no third-party repo or GPG key on a host about to be hardened. Reverses an earlier recommendation |
| Bundle delivery | **Copy** `deploy/compose/` (44 KB), record the commit | vs ~460 MB clone for 32 KB of files; re-running the playbook is the update path, not host-side `git pull` |
| Image pin | **Sync-point method** → `sha-96ae141` | Jeff's idea, and better than "newest tag": the image must match the *bundle*, not upstream's latest. Scripted as `scripts/resolve-image-tag.sh` |
| Rehearsal hostname | `buzz-vm.test:3000` | `.test` is reserved for this; `.local` collides with macOS mDNS |
| Front-end serving | Relay serves both bundles from directories | Mirrors `just web` / `just admin`; what production will do |
| Compose override name | `compose.cohort.yml`, **not** `compose.override.yml` | Docker auto-loads the latter for bare commands but `run.sh` passes explicit `-f` and would ignore it — two different stacks depending on the command typed |
| SOP platforms | macOS and Windows, via Git Bash | Windows-specific parts are entirely unrun |

Deferred, deliberately: the production owner identity (`RELAY_OWNER_PUBKEY` custody), the production
hostname, and everything hardening.

## Corrections made — things that were wrong

Recorded because each was written down confidently before being tested.

1. **"No command seeds the `communities` table."** #19's original DoD said to record the command
   used. There is none — the relay seeds from `RELAY_URL` at startup. Confirmed at
   `crates/buzz-relay/src/main.rs:259-291` and `crates/buzz-db/src/lib.rs:1369`.
2. **"NIP-11 and health endpoints are tenant-gated."** They are not. `/`, `/health` and
   `/_liveness` all return **200 with a bogus `Host`**. Only the **WebSocket upgrade** is gated:
   101 for the seeded host, 404 `relay: no community is configured for this host` otherwise. This
   matters — verifying binding with NIP-11 passes on a relay that refuses every client.
3. **"Ubuntu's Compose is too old."** It is 2.40.3. This reversed the Docker-source decision.
4. **`BUZZ_WEB_DIR` alone does nothing visible.** Also needs `BUZZ_SERVE_GIT_WEB_GUI=true` to serve
   at `/`; without it the bundle only answers `/invite/<code>`.
5. **The admin dashboard is read-only** — moderation reports and product feedback. It cannot manage
   users or the roster. Roster management is the `buzz-admin` CLI only.

## Verified vs not

**Verified by execution:** Docker install and versions; all five containers healthy; community
seeded as `buzz-vm.test:3000` with the port preserved; owner bootstrapped; WS 101 vs 404 including
from the host machine; health/NIP-11 not gated; web bundle at `/` and `/invite/<code>`; admin at
`/reports` and `/api/admin/v1/reports` with **403** on the wrong Host; `scp` transfers preserving
dotfiles and the executable bit; `resolve-image-tag.sh` happy path.

**Not verified:** membership gating by behaviour (no non-rostered key refused — it happens during
NIP-42 auth, *after* the 101, so the upgrade result proves nothing about it); the desktop app (never
built or launched; the `wss://` behaviour is read from source only); SOP Steps 1–4 as written on a
clean machine; **every Windows instruction**; `resolve-image-tag.sh`'s bundle-differs guard and
walk-back branch; anything about disk capacity or behaviour under load.

## Waiting on you

### Four GitHub artifacts drafted but NOT posted

You authorised posting mid-session; a permission classifier blocked the subagent's writes and my own
`gh` attempt was never made. All four are corrected and ready in `bodies/`:

| File | Target | Action |
|---|---|---|
| `adr24-ratification-comment.md` | Comment on **#24** | Proposes Ansible + noble + compose for ratification. Do **not** fill in Decision outcome or remove `needs-decision` |
| `t2-revised.md` | Replace body of **#19** | Corrects the non-existent seeding command and the Host-header error |
| `t0-revised.md` | Replace body of **#17** | Narrows scope under the mirror/configure split |
| `REVISIONS.md` | Basis for explanatory comments | `gh issue edit` silently replaces bodies; post a comment saying what changed |

```bash
gh issue comment 24 --repo launchpad-26/buzz --body-file bodies/adr24-ratification-comment.md
gh issue edit 19 --repo launchpad-26/buzz --body-file bodies/t2-revised.md
gh issue edit 17 --repo launchpad-26/buzz --body-file bodies/t0-revised.md
```

The #24 draft's footer says nothing has been measured. **That is now stale** — fix it before posting.

### Decisions only you can make

- **#21** sizing ADR — the evidence is in.
- **Production owner identity.** Whoever holds the `RELAY_OWNER_PUBKEY` secret controls the relay.
  Personal identity vs an escrowed cohort key. Easier now than after members are rostered.
- **The production hostname**, single value. A bare name and its `www.` alias are separate
  communities with separate data.
- **Hardening documentation shape.** The SOP is the automation's source of truth, and it covers
  **no** hardening at all — in fact it deliberately enables root SSH, password auth and a known
  password. #29 and #30 would *break* the SOP's own instructions. Likely wants a separate
  `prod-hardening-SOP.md` that says which dev steps it supersedes. Several #5 tasks are blocked on
  ADRs #24, #26, #27 anyway.
- **Milestone M0** date has passed.

## Known defects in this folder

1. **`virtual-box/build-vps-clone.sh` had a hardcoded password hash.** Replaced with a required
   `PWHASH` environment variable before committing, since committing it would breach #17's own DoD.
   The script's behaviour is otherwise unchanged and **has not been re-run** since.
2. **`OVA` pointed at a temp scratch directory** from an earlier session. Now an overridable
   variable defaulting to `~/vm-images/noble.ova`.
3. **`seed.sample/` was not copied into the repo.** It is a stale build output the script never
   reads, and it contained a real SSH public key. Gitignored.
4. **`bodies/`, `prd0-breakdown.md`, `prd3-breakdown.md` were not copied.** Issue-drafting
   artifacts, not deployment code. They stay in `~/vps-clone`.
5. The script still creates only the **2222** forward, not 3000.
6. **Path naming:** this was staged as `launchpad/dev/deploy/` in `AGENTS.md`, but committed to
   `launchpad/deploy/` as instructed. `ansible/` is the one thing here that is *not* dev-only — it
   targets production too — so if the `dev/` split returns, `ansible/` probably belongs outside it.

## Resuming

```bash
VBoxManage list runningvms                       # is it still up?
VBoxManage startvm vps-clone-noble --type headless
ssh -p 2222 root@127.0.0.1
cd /opt/buzz/compose && docker compose ps
```

Then read [`runbooks/dev-deployment-SOP.md`](./runbooks/dev-deployment-SOP.md), and
[`runbooks/relay-build-list.md`](./runbooks/relay-build-list.md) for why the pieces behave as they
do. `AGENTS.md` carries the rules; `ansible/README.md` carries what the roles must reproduce.
