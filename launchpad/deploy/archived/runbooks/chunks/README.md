# Chunk map — the chunked Buzz deployment

**Start here instead of the SOP.** This index exists so that "which chunk do I run, and what has to
have happened first" is answerable without opening a 3,000-line document.

## Three layers, one direction of authority

The deployment is cut into twelve numbered **chunks**, each independently runnable and each
idempotent. Three layers describe them, and they are not interchangeable:
[`../dev-deployment-SOP.md`](../dev-deployment-SOP.md) is the **specification** and holds *all* of
the reasoning — why a value is what it is, which trap each command avoids, what the expected output
means; the **chunk docs** in this directory (`NN-<name>.md`) are deliberately thin operational
wrappers that hold only the operation — preconditions, the exact command, the verification, the
rollback, and the non-obvious traps, each citing the SOP step the reasoning lives in; and the
**Ansible roles** under [`../../ansible/`](../../ansible/) plus the shell scripts in
[`../../virtual-box/`](../../virtual-box/) and [`../../scripts/`](../../scripts/) are the
implementation. A chunk doc that re-explains reasoning has become a third document to keep in sync,
which is the problem the split solves. **When implementation and SOP disagree, THE SOP CHANGES
FIRST** — then the chunk doc, then the role. Never the other way round
([`../../ansible/README.md`](../../ansible/README.md), [`../README.md`](../README.md)).

The runnable index is [`../../deploy`](../../deploy) — `./deploy list` prints the same twelve rows
from its own `CHUNKS` array. That array is implementation; this file is its documentation. If they
disagree, the SOP settles it and both get corrected.

## The chunks

| # | Name | Where | What it does | SOP steps | Artifact |
|---|---|---|---|---|---|
| 00 | preflight ([doc](./00-preflight.md)) | HOST | Every host-side check that must pass before the VM is built; read-only, changes nothing | 0.2, 0.3, 0.4, 1, 3.2 (plus a warning-only look ahead at Step 10) | `virtual-box/preflight.sh` |
| 01 | base-image | HOST | Downloads and checksums the Ubuntu 24.04 noble cloud OVA into `~/vm-images/` | 1 | `virtual-box/fetch-image.sh` |
| 02 | vm-create | HOST | **Destroys and rebuilds** `buzz-dev` — import, resize, CPU/RAM, NAT forwards, cloud-init seed, first boot | 2, 3, 4 | `virtual-box/build-vps-clone.sh` |
| 03 | host-dns ([doc](./03-host-dns.md)) | HOST | Job 1 (required): the two `/etc/hosts` entries. Job 2 (`--ca-only`, **experimental TLS profile only**): fetch Caddy's local CA and print the trust command | 10, and the client-address half of 12.3 | `virtual-box/host-dns.sh` |
| 04 | docker ([doc](./04-docker.md)) | TARGET | Installs `docker.io`, `docker-compose-v2`, `containerd` from the Ubuntu archive; asserts Compose ≥ 2.24.4 | 5, 5.1, 5.2 (5.3's snapshot is host-side) | `ansible/playbooks/04-docker.yml` |
| 05 | bundle ([doc](./05-compose-bundle.md)) | TARGET | Copies the seven upstream `deploy/compose/` files to `/opt/buzz/compose/` and records the resolved image pin | 6 (6.1, 6.2, 6.3, 6.4) | `ansible/playbooks/05-bundle.yml` |
| 06 | config ([doc](./06-config.md)) | TARGET | Generates secrets on the target, renders `.env` (0600, root) and the cohort overrides — `compose.cohort.yml` and the Caddyfile the experimental profile mounts | 7 | `ansible/playbooks/06-config.yml` |
| 07 | up ([doc](./07-up.md)) | TARGET | Brings the stack up on the compose set `buzz_compose_files` resolves to and verifies the community, the WebSocket upgrade and all four surfaces | 8, 9 | `ansible/playbooks/07-up.yml` |
| 08 | desktop ([doc](./08-desktop.md)) | MANUAL | Builds and launches the Tauri desktop app as the owner and connects it to `ws://buzz-vm.test:3000`. GUI — not scriptable | 12 | `runbooks/chunks/08-desktop.md` |
| 09 | members | TARGET | Approves pubkeys on the relay roster, one at a time | 11 | `ansible/playbooks/09-members.yml` |
| 10 | harden | TARGET | The dev-VM hardening subset: SSH policy, ingress firewall, kernel parameters, unattended upgrades, Docker daemon | 13 | `ansible/playbooks/10-harden.yml` |
| 11 | verify | BOTH | The full verification suite — the checklist, run as assertions rather than by eye | 17 | `scripts/verify.sh` |

`HOST` = your own Mac. `TARGET` = inside the VM (or the VPS), via Ansible. `MANUAL` = a person
follows a runbook. `BOTH` = checks in each place.

Chunk docs follow the `NN-<name>.md` convention in this directory. **All twelve now exist:**
[00](./00-preflight.md) · [01](./01-base-image.md) · [02](./02-vm-create.md) ·
[03](./03-host-dns.md) · [04](./04-docker.md) · [05](./05-compose-bundle.md) ·
[06](./06-config.md) · [07](./07-up.md) · [08](./08-desktop.md) · [09](./09-membership.md) ·
[10](./10-harden.md) · [11](./11-verify.md).

## Ordering

The numbers are the running order, with three constraints the numbers do **not** express.

```
00 preflight ─► 01 base-image ─► 02 vm-create ─┬─► 04 docker ─► 05 bundle ─► 06 config ─► 07 up ──┐
                                               │                                                  │
               03 host-dns  job 1 (/etc/hosts) ─┘  (any time; needed before any browser check)     │
                                                                                                   │
   ┌───────────────────────────────────────────────────────────────────────────────────────────────┘
   │
   ├─► 03 host-dns  job 2 (--ca-only + trust the CA)   ← EXPERIMENTAL TLS PROFILE ONLY; requires 07 (Caddy mints the CA on first start)
   ├─► 08 desktop (manual)                             ← needs only 03 job 1; the app cannot use the TLS profile at all (#108)
   ├─► 09 members                                      ← REQUIRES 07: buzz-admin resolves the community seeded at startup
   │
   ├─► snapshot `buzz-working`                         ← SOP 13.1. Two of chunk 10's plays can lock you out
   ├─► 10 harden
   └─► 11 verify
```

The three constraints, stated plainly:

1. **Chunk 03 runs once on the default path, twice on the experimental one.** Job 1 (the
   `/etc/hosts` entries) can run before the VM exists and is all the plaintext default needs. Job 2
   (`--ca-only`) belongs to the Caddy + `tls internal` profile and **cannot run until chunk 07 has
   started Caddy at least once**, because the local root CA does not exist until then — before that
   the script exits `3` (SOP Step 10; hardening-spec.md §A.3).
2. **Chunk 09 cannot run until chunk 07 has seeded the community.** There is no seeding command —
   the relay creates the community from `RELAY_URL` at startup, and the roster tool looks it up. Run
   it early and you get `RELAY_URL host '...' is not mapped to a community` (SOP Step 11;
   [`../relay-build-list.md`](../relay-build-list.md)).
3. **Chunk 10 must run only after a snapshot exists.** SOP Step 13.1 names it `buzz-working`, and it
   is the only undo for the SSH-policy and firewall plays: `./deploy snapshot buzz-working`.

The desktop app in chunk 08 no longer depends on chunk 03 job 2, and that is a decision rather than a
simplification: the app cannot connect to a relay behind a private CA at all (issue #108), so the
dependency was replaced by using the plaintext default rather than satisfied.

Two softer orderings worth knowing: chunk 08 is the last thing that needs the pre-hardening state
(nothing in Step 13 breaks it, but debugging a GUI client against a freshly firewalled host is a
worse experience than doing it in the other order), and chunk 11 is the finish line — SOP Step 17 is
"the deployment is finished when every one of these passes."

## For agents

Full clean deploy, from a machine that has nothing:

```bash
cd /Users/jeff/group-build-project/buzz/launchpad/deploy

./deploy list                     # the same twelve rows, from the entrypoint itself

./deploy run 00                   # host checks; must report 0 failed
./deploy run 01                   # download noble.ova
./deploy run 02                   # DESTROYS and rebuilds buzz-dev
./deploy run 03                   # job 1 only: /etc/hosts entries
./deploy run 04                   # Docker

# 05 and 06 need the resolved image pin, which `./deploy run` cannot pass.
# Resolve it once and drive these two directly (see 05-compose-bundle.md; 06's
# -e interface is inferred from that doc's "keep $IMAGE" note -- confirm it when
# 06-config.yml lands).
OUT=$(scripts/resolve-image-tag.sh /Users/jeff/group-build-project/buzz HEAD)
TAG=$(printf '%s\n' "$OUT" | sed -n 's/^BUZZ_IMAGE=//p')
DIGEST=$(printf '%s\n' "$OUT" | sed -n 's/^# digest:[[:space:]]*//p')
SYNC=$(printf '%s\n' "$OUT" | sed -n 's/^# sync point:[[:space:]]*//p')
IMAGE="${TAG%:*}@${DIGEST}"
( cd ansible && ansible-playbook playbooks/05-bundle.yml --limit dev-vm \
    -e "buzz_image=$IMAGE" -e "buzz_image_tag=$TAG" -e "buzz_sync_commit=$SYNC" )
( cd ansible && ansible-playbook playbooks/06-config.yml --limit dev-vm \
    -e "buzz_image=$IMAGE" )

./deploy run 07                   # stack up (compose.yml + compose.cohort.yml), verified

# ONLY under the experimental Caddy + `tls internal` profile: chunk 03 again, now
# that Caddy has minted its CA. The script prints this second command with the
# absolute path filled in; it never runs it itself. Skip both lines on the
# default path — there is no TLS to trust, and trusting this CA would not make
# the desktop app work anyway (#108).
# ./virtual-box/host-dns.sh --ca-only
# sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain \
#   ~/vm-images/buzz-dev-caddy-root.crt

./deploy run 09                   # roster
./deploy snapshot buzz-working    # SOP 13.1 — the undo for chunk 10
./deploy run 10                   # hardening subset
./deploy run 11                   # verification suite
```

Chunk 08 is then done by hand: [`08-desktop.md`](./08-desktop.md).

**Every chunk is safe to re-run.** They are idempotent by requirement, not by luck — a second run of
an Ansible chunk must report `changed=0`, and a second run of a host script must report nothing to
change (`../../ansible/README.md` "Convergence", Ruling 11, issue #36). The single exception to
"safe" is **chunk 02, which destroys the VM**; it is re-runnable in the sense that it converges on a
freshly built machine, not in the sense that you can run it casually.

`./deploy check <chunk>` runs an Ansible chunk with `--check --diff` and changes nothing. It is only
meaningful after that chunk has succeeded once — against a never-provisioned host, tasks that read
state fail because the state does not exist yet.

### Known limitations of `./deploy run all`

`run all` runs `00 01 03 04 05 06 07 09 10 11`, skipping 02 (destructive) and 08 (GUI). Three gaps
follow, all of them things you would otherwise discover at the failure:

- It **does not build the VM** (02 is skipped), so it assumes `buzz-dev` already exists and answers
  on `127.0.0.1:2222`.
- It **fails at 05**, and would fail at 06, because neither playbook has a default for
  `buzz_image` — deliberately, since a guessed pin pairs a relay binary with configuration it was
  never built against (`ansible/roles/compose_bundle/defaults/main.yml`). `./deploy run` has no way
  to pass `-e`. Use the explicit `ansible-playbook` lines above.
- It runs **03 once, with no arguments**, and **10 with no snapshot**. The first is now correct for
  the default path (job 1 is all it needs); under the experimental TLS profile the CA is still never
  fetched and never trusted, and the entrypoint cannot express `./deploy run 03 --ca-only`. The
  snapshot has no entrypoint form either. Both remain manual steps in the sequence above.

Treat `run all` as a convergence check on an already-deployed host (run it twice; the second run
should change nothing), not as the clean-deploy path.

## Scope

**The chunks cover SOP Steps 0–13, plus Step 17 as the verification suite.**

**SOP Steps 14, 15 and 16 — the AI agent — are deliberately out of scope.** No chunk implements
them: no chunk generates the agent identity, writes `.env.agent`, adds the `agent` service, or adds
the agent to a channel. If you want the agent, follow those three SOP steps by hand after chunk 11.

Two consequences of that boundary:

- Chunk 11 cannot assert SOP Step 17's agent items — the `agent` container in item 2, items 19–26
  (including item 24, the SOP's own acceptance test), and item 29's `buzz-agent-working` restore.
  That snapshot never exists on this path; the last snapshot the chunks take is `buzz-working`.
- Chunk 08 still creates a channel, because checklist item 18 wants one and it is the cheapest proof
  that publishing works — but its `agent-test` name is inherited from a step nothing here runs.

**Including Step 13 hardening knowingly overrides [`../../AGENTS.md`](../../AGENTS.md) rule 5**,
which states that firewall, SSH policy, AppArmor, unattended-upgrades and service-identity roles
deliberately do not exist yet because they are issues #29–#34 under PRD #5, ahead of PRD #2's
deadline. That override was an explicit decision by the repo owner and is stated here rather than
silently taken. It is also the narrower reading of the two: what chunk 10 implements is the **dev-VM
subset** described at the top of SOP Step 13, *not* the full production baseline in
[`../hardening-spec.md`](../hardening-spec.md) — which additionally requires `DOCKER-USER` rules,
default-deny **egress**, split internal/external container networks, module blacklisting, systemd
resource limits, `auditd` and sudo I/O logging, authenticated time, and Part E backups. Do not read
a green chunk 10 as "production-hardened."

## Two topologies: the supported one and the research one

**The default is the SOP's own plaintext path — `ws://buzz-vm.test:3000`, no Caddy — and it is the
daily supported development environment.** The relay is reached directly on 3000 through a NAT
forward `127.0.0.1:3000 → guest 3000`, the community is `buzz-vm.test:3000` with the port preserved,
and the desktop app connects to that address. This is the path the SOP documents in Steps 7.3, 10 and
12.3, and it was verified working on 2026-08-11.

**Caddy + `tls internal` is retained as an explicitly experimental, opt-in profile — and it is not
desktop-app compatible.** It remains the only place we exercise the admin vhost split, the
reverse-proxy configuration, and `wss://` end to end, and it is the groundwork for a real public
certificate later, so it is kept rather than deleted. Under it Caddy runs with `tls internal`,
`compose.caddy.yml`'s `ports: !reset []` removes the relay's published 3000 entirely, and the host
forwards 8080→80 and 8443→443 because macOS forbids a non-root bind below 1024 — guest-side ports
stay 80/443, so container configuration is identical to production. It is specified by
hardening-spec.md §A.3 and §B2.

What it cannot do is carry the desktop app. The app's native WebSocket (`tokio-tungstenite`, built
with `rustls-tls-webpki-roots`) trusts only the compiled-in Mozilla root set, while its HTTP bridge
(`reqwest` via `rustls-platform-verifier`) uses the macOS keychain. Against a private CA the HTTP
half succeeds and every WebSocket dial fails, so the app reads and cannot send — a trust-store
difference between two clients in one binary, established with `openssl s_client` and packet capture,
tracked as issue **#108**. Trusting the CA in the System keychain does not change it. A one-line
dependency change resolves it and was deliberately reverted, because it edits an upstream-owned file
against this fork's governance rule and the intended trust behaviour was never agreed with upstream.

The canonical dev values, and the one-line switch between the two profiles, are in
[`../../ansible/inventory/group_vars/dev.yml`](../../ansible/inventory/group_vars/dev.yml);
`buzz_compose_files` is derived from `buzz_tls_mode` in
[`../../ansible/inventory/group_vars/all.yml`](../../ansible/inventory/group_vars/all.yml).
**Production is never `none`** — `compose.caddy.yml`'s `ports: !reset []` is §B2's required control
1 and `ufw` cannot substitute for it, because Docker's rules are evaluated first.

**The Ansible roles still encode the experimental topology, and that is an open debt.** `relay_config`
renders and validates a Caddyfile unconditionally, `compose.cohort.yml.j2` always emits a `caddy:`
service block, and both `relay_config` and `relay_up` assert that the relay publishes no host port and
probe every surface over `https://…:443`. All of that is correct for the experimental profile and
fails on the plaintext default, where the relay legitimately publishes 3000. Re-scoping those
assertions changes a stated security control, so under "the SOP changes first" hardening-spec.md §B2
must carry the dev carve-out before the roles follow. Until then chunks 06 and 07 run the experimental
profile, and the default path is driven by SOP Steps 7–9 directly.

## Provenance

Following the SOP's own honesty convention, and split so the two halves cannot be confused: what has
actually been executed, then what has not. Nothing in the second list is a claim of verification.

**Read every TLS item below as "verified for the experimental profile".** The Caddy + `tls internal`
topology is verified for the relay, browsers and `curl`, and explicitly **not** for the desktop app.
The plaintext default's own verification is the first list item.

### Verified by execution on 2026-08-11 — the default path

- **`ws://buzz-vm.test:3000` works.** The plaintext relay path, as the SOP documents it in Steps 7.3,
  10 and 12.3, was verified working on this date. It is the reason it is the default rather than the
  fallback. Its verification predates the chunked Ansible path, so it was executed from the SOP's own
  commands and **not** through chunks 06/07 — see "Still not verified".

### Verified by execution on 2026-08-12 — the experimental TLS profile

On a freshly built `buzz-dev` (1 vCPU / 1.9 GiB / 496 MB swap / 20 GB disk), macOS Intel host,
image `ghcr.io/block/buzz:sha-96ae141`, restored to the `pristine` snapshot first:

- **Chunks 02, 04, 05, 06, 07 run clean, in order, from a pristine VM.** Chunk 07 finishes
  `ok=22 changed=0 failed=0` on a converged host.
- **Convergence holds for every Ansible chunk.** A second run of 04, 05, 06, 07, 09 and 10 each
  reports `changed=0` — the requirement from `ansible/README.md` "Convergence", Ruling 11 and issue
  #36. Chunk 10's second run is `ok=97 changed=0 failed=0`.
- **The TLS profile works end to end for relay, browser and `curl` — but not for the desktop app.**
  Caddy with `tls internal` fronts the relay, the community seeds as `buzz-vm.test:8443` with the port
  preserved, and a WebSocket upgrade through Caddy returns `101`. The desktop app is the documented
  exception (issue #108, below).
- **`compose.caddy.yml`'s `ports: !reset []` provably closes 3000** — `docker compose ps` shows the
  relay with `3000/tcp` exposed and **no published host port**; only Caddy publishes 80/443.
- **All four surfaces answer through Caddy**: web bundle `200 text/html`, NIP-11 `200
  application/json`, admin dashboard `200`, admin API `200` on the admin host and **`403` on the
  ordinary host**.
- **Three refusal layers behave differently and all three were observed**: `101` on the correct
  Host; `000` (TLS handshake refused) for an unknown SNI at Caddy, proving no wildcard site block;
  and `relay: no community is configured for this host` only when Caddy is bypassed on the Docker
  network.
- **Docker from the Ubuntu archive**: docker.io 29.1.3, Compose 2.40.3, containerd 2.2.1 — above the
  2.24.4 floor `!reset` needs.
- **`chunk 11` (`./deploy verify`) reports 34 passed, 0 failed, 0 skipped** on the fully deployed and
  hardened VM. Run before chunk 10 it reports 19 passed / 0 failed / 2 skipped, the two skips being
  the hardening block — assertions whose chunk has not run skip rather than fail, so the suite is
  useful part-way through.
- **Swap never touched** throughout.
- **Chunk 10 passes: `ok=99 changed=3 failed=0`.** This is the first time SOP Step 13 has been
  executed anywhere — it was written from source and had never been run. `sshd -T` reports
  `permitrootlogin no`, `passwordauthentication no`, `kbdinteractiveauthentication no`; `ssh
  root@...` is refused with `Permission denied (publickey)`; ufw is active with `deny (incoming)`;
  the relay still answers `200` afterwards.
- **Getting chunk 10 to pass took five fixes**, each a real defect rather than a tuning issue: the
  sshd policy was assembled with `join('\n')` inside a YAML block scalar and reached sshd as one line
  containing a literal `\n`; the sysctl file lost to `/usr/lib/sysctl.d/99-protect-links.conf` on name
  order; `ansible_managed` was used inside `copy: content:`, where it is never defined; the
  Allowed-Origins check used a `'\\1'` backreference that emitted a literal `\1`; and the documented
  rollback snapshot was named `pre-harden` while every other file said `buzz-working`.
- **The SSH role's safety design was exercised for real.** When the malformed policy was written,
  `sshd -t` rejected it, the role restored the previous state, refused to reload, and reported
  "sshd is still running its old configuration — you are not locked out" (`rescued=1`). The lockout
  guard is not theoretical.
- **Chunk 03 runs clean, both jobs.** `./virtual-box/host-dns.sh --ca` does job 1 and job 2 in a
  single pass with **one** `sudo` prompt, and `--ca-only` needs no local `sudo` at all (its only
  `sudo` runs inside the VM, where it is passwordless). The script prints the `security
  add-trusted-cert` command and correctly does not run it. Afterwards: both `/etc/hosts` entries are
  present under the script's marker comment, the DNS cache is flushed, and `Caddy Local Authority -
  2026 ECC Root` is in the System keychain. Note that `./deploy run 03` passes no arguments and the
  script defaults to `DO_DNS=1` / `DO_CA=0`, so the entrypoint runs **job 1 only** and the CA is never
  fetched.
- **The CA trust was proven with no `-k` and no `--cacert`**, which is the entire point of the check:
  `https://buzz-vm.test:8443/` returns `200` with `ssl_verify_result=0` (0 = genuinely verified, not
  bypassed), the admin vhost `https://admin.buzz-vm.test:8443/reports` returns `200`, and a WebSocket
  upgrade over the trusted TLS returns `101`.
- **Chunk 08 is PARTIALLY verified — the build and the launch, nothing past them.** On an Intel Mac
  **without Hermit** (Node v22.23.1, pnpm 11.4.0, `pnpm install` reporting `Already up to date`),
  `pnpm -C desktop tauri dev` built in **6m25s** and launched, and the startup line `buzz-desktop:
  configured identity pubkey <64 hex>` appeared with a value **identical** to `RELAY_OWNER_PUBKEY` in
  the relay's `.env` — SOP Step 12.4's ownership check works exactly as documented. Two hard
  requirements the doc had missed surfaced here: `brew install cmake` (`aws-lc-sys`, in both
  `Cargo.lock` files, builds AWS-LC through CMake) and `. "$HOME/.cargo/env"` in any shell opened
  before rustup was installed. The GUI half is *not* verified — see the second list.
- **Rebuilding the VM invalidates `~/.ssh/known_hosts` every time.** Hit for real on 2026-08-12: the
  deleted `vps-clone-noble` had used port 2222 and left three key entries behind, so ssh refused with
  `REMOTE HOST IDENTIFICATION HAS CHANGED` rather than prompting. Now a chunk 02 trap.
- **The `sshd` ordering trap is real, not theoretical.** A pristine guest carries
  `01-dev-parity.conf`, `50-cloud-init.conf` **and** `60-cloudimg-settings.conf`, with `sshd -T`
  reporting `permitrootlogin yes`. A `99-`prefixed hardening file would lose to both `50-` and
  `60-`, which is why chunk 10 uses `00-hardening.conf` *and* deletes the parity file.

Four traps were found by execution and are now encoded in the chunks; none is in the SOP:

1. **`--http1.1` is mandatory on the WebSocket check.** Over TLS curl negotiates HTTP/2 via ALPN,
   and HTTP/2 removed `Connection: Upgrade` — the same request returns `101` with the flag and `200`
   without it, so the check passes while proving nothing. The SOP cannot have this trap because it
   tested plaintext HTTP/1.1 on port 3000.
2. **Capture only the status code from that check.** A successful upgrade leaves the socket open and
   curl emits raw WebSocket frames, which are not valid UTF-8; Ansible then fails the task with a
   surrogate-decoding error despite correct relay behaviour.
3. **Caddy declares no healthcheck**, so `docker compose ps --format json` reports `Health: ""` for
   it while `up --wait` prints `Healthy`. Asserting `healthy` for every service fails on a good stack.
4. **SOP Step 17 item 27's secret scan fails on a clean checkout.** `git grep -nE 'sk-or-v1-|OPENROUTER_API_KEY=sk'`
   matches the SOP's own prose, `buzz-agent/README.md`, and a desktop test fixture that exists
   *to test secret redaction*. Chunk 11 scopes the scan and matches real key shapes instead.

### Still not verified
- **The hardening is the dev-VM SUBSET, and a green chunk 10 must not be read as more.** Step 13 has
  now run (see above), but `hardening-spec.md`'s production baseline remains unimplemented:
  `DOCKER-USER` rules, default-deny **egress** (§C4), the split internal/external container networks
  (§B5), module blacklisting, systemd resource limits (§C6), `auditd` and sudo I/O logging (§C8),
  authenticated time (§C9), the MinIO service account (§B4), Caddy rate limiting (§B9), image digest
  pinning (§B12) and Part E backups.
- **SOP Steps 14–17 remain unrun.** 14–16 are out of scope by decision. Step 17 exists here as chunk
  11's assertions, but the agent-dependent items (2's `agent` container, 19–26 including item 24, the
  SOP's own acceptance test, and 29's `buzz-agent-working` restore) can never pass on this path.
- **No break-glass exists, and nothing here is safe for the VPS without it.** Every rollback in these
  chunks is a VirtualBox snapshot. `hardening-spec.md` §C1 requires cloud serial console access plus a
  sealed out-of-band credential, built *and tested*, before chunk 10 runs anywhere near production.
- **No production inventory or `group_vars/prod.yml` exists**, so five required-but-empty variables in
  `group_vars/all.yml` (`buzz_domain`, `buzz_relay_url`, `buzz_acme_email`, `buzz_max_connections`,
  `buzz_backup_target`) have nowhere to be set. No play can run against a VPS today.
- **A client HAS now connected successfully, as the owner — over the HTTP bridge only.** Observed
  2026-08-12 against the experimental TLS profile after setting `BUZZ_DEV_KEYRING_SERVICE`: the app
  authenticated as `RELAY_OWNER_PUBKEY` (`2cf82270…`), the relay logged sustained NIP-98 `HTTP bridge
  request` entries under that pubkey and **no refusals**, and onboarding produced 6 kind:39000
  channels with 6 matching kind:39002 memberships. Still not done: **posting a message** — the event
  store holds no kind:9 or kind:40002, so SOP Step 17 item 18 remains unproven and chunk 08 is not yet
  fully closed. In hindsight this is the exact signature of #108: reads travel over the bridge, writes
  need the WebSocket, and the WebSocket never connected.
- **Neither chunk 06 nor chunk 07 has been executed against the plaintext default.** They currently
  render a Caddyfile unconditionally, always emit a `caddy:` service block, and assert the relay
  publishes no host port — all correct for the experimental profile and all failing on the default,
  where the relay legitimately publishes 3000. See "Two topologies" above; the roles follow once
  hardening-spec.md §B2 carries the dev carve-out.
- **The `relay` NAT forward (`127.0.0.1:3000 → guest 3000`) has not been exercised**, and no VM has
  been rebuilt since it was added to `virtual-box/build-vps-clone.sh`. An existing VM can take it live
  with `VBoxManage controlvm buzz-dev natpf1 "relay,tcp,127.0.0.1,3000,,3000"`.
- **`scripts/verify.sh` (chunk 11) still hardcodes the three-file invocation** and asserts that only
  Caddy's 80/443 are published, so its 34-assertion pass is a result about the experimental profile.
  It has never run against the default path.
- **Transport details, and the claim they were later corrected into.** Observed 2026-08-12: the
  desktop app reached the relay over `wss://buzz-vm.test:8443` and the relay logged HTTP bridge
  activity proxied through Caddy from `172.18.0.6` — Caddy's address inside `buzz-net` — so *a* TLS
  handshake against the `tls internal` certificate did complete. The conclusion drawn at the time,
  that "Tauri's Rust TLS stack accepts the Caddy local CA once it is trusted in the System keychain"
  and that chunk 03's `curl` check was therefore sufficient, was **too broad and has been retracted**.
  It held for the HTTP bridge only (`reqwest` via `rustls-platform-verifier v0.7.0`, which does
  consult the keychain). The app's native WebSocket is built with `rustls-tls-webpki-roots` and trusts
  only the compiled-in Mozilla roots, so every `wss://` dial against the private CA failed while the
  bridge kept working — established afterwards with `openssl s_client` and packet capture, tracked as
  issue **#108**. That is the finding that made the plaintext path the default.
- **Membership refusal is PROVEN, closing issue #19's last open claim.** Observed 2026-08-12: a key
  that was not on the roster authenticated and the relay logged
  `not a relay member … 403 relay_membership_required`, then closed the connection. Refusal happens
  during NIP-42 auth, *after* the WebSocket upgrade, exactly as the SOP predicted — so a `101` really
  does prove nothing about the roster, and the roster really is enforced. The SOP's "whether an
  unapproved key is actually refused is untested" note can be retired.
- **`tauri dev` collides with any existing Buzz install on the same Mac, and this is not a footnote.**
  Observed 2026-08-12: the app authenticated with an identity from the shared `buzz-desktop-dev`
  macOS keyring rather than the `BUZZ_PRIVATE_KEY` owner identity it had logged at startup, and
  carried a community from a previous install. `BUZZ_PRIVATE_KEY` sets the *configured* identity —
  the startup line proves it loads — but does not override a saved community's stored identity. The
  data directory (`~/Library/Application Support/xyz.block.buzz.app`) is shared for the same reason.
  The recipe that isolates app identifier, keyring and ports is `just desktop-standalone`, which is
  **unusable here** because it unsets `BUZZ_PRIVATE_KEY` immediately before launching.
  **RESOLVED the same day:** export `BUZZ_DEV_KEYRING_SERVICE=buzz-desktop-dev.buzzvm` before
  launching (`desktop/src-tauri/src/app_state_keyring.rs:13` — any value prefixed
  `buzz-desktop-dev.` is honoured, anything else silently falls back to the shared default). With it
  set, the app authenticated as `RELAY_OWNER_PUBKEY` with zero refusals; the stale identity's last
  appearance predates the relaunch. No keychain entry was deleted and the WebKit store was not
  touched. It does **not** namespace `localStorage` (the community list, at
  `~/Library/WebKit/buzz-desktop`, keyed by process name) — that stays shared.
- **SOP Steps 1–4 have never been run as written** on a clean machine, and
  `virtual-box/build-vps-clone.sh` has not been re-run since its hardcoded password hash was
  replaced by the required `PWHASH` variable. Chunk 02 wraps exactly that script.
- **`resolve-image-tag.sh`'s failure branches are unexercised** — only the happy path has run. Its
  bundle-differs guard and its first-parent walk-back are unverified (`../../scripts/README.md`).
- **Windows is not covered at all.** The chunked path is macOS-only by construction (chunk 00 fails
  on anything else, and `build-vps-clone.sh` builds the seed with `hdiutil`). The SOP's **On
  Windows** blocks remain unrun and no chunk implements them.
- **Nothing about disk capacity or behaviour under load.** Unchanged from the SOP: this VM's disk is
  deliberately smaller than production's, and the relay's default 10,000 connections at 1,000
  buffered messages each have never been exercised (issue #39).

Whoever runs these first: correct the SOP where it is wrong, move what you confirmed out of this
list, and keep it honest. It is what tells the next reader how far to trust each chunk.
