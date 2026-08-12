# Build list — getting the Buzz relay running inside the VM

Prerequisites, in order, for #18 (capacity measurement) and #19 (Host/membership rehearsal).
Derived by reading `deploy/compose/` and `crates/` in `/Users/jeff/group-build-project/buzz`,
then **executed end to end on the spec-matched VM on 2026-08-11**.

## Verified run — 2026-08-11

Full stack reached healthy on 1 vCPU / 1.9 Gi with **swap never touched**. Not a substitute for
#18's report: this was a manual learning pass, and the VM's disk is far smaller than the VPS's.

Config: `BUZZ_IMAGE=ghcr.io/block/buzz:sha-96ae141` (resolved by
`scripts/resolve-image-tag.sh`), `RELAY_URL=ws://buzz-vm.test:3000`,
`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`, `BUZZ_AUTO_MIGRATE=true`, Docker from the Ubuntu archive
(`docker.io` 29.1.3, Compose **2.40.3**, containerd 2.2.1).

| Stage | Mem used | Peak | Swap | Disk used |
|---|---|---|---|---|
| Bare VM, no Docker | 296 MB | — | 0 | 2.2 G |
| Docker installed, idle | 449 MB | — | 0 | 2.8 G |
| `docker pull` ×5 images | 469 MB | **474 MB** | **0** | 3.9 G |
| Migration + first start | 560 MB | **563 MB** | **0** | 3.9 G |
| Steady idle | 573 MB | — | **0** | 3.9 G |

`./run.sh start` reached all-healthy in **18 seconds**. Per-container at idle: relay **9 MB**,
postgres 51 MB, redis 3 MB, minio 92 MB. Images total 1.145 GB on disk. Host overhead is
`dockerd` 78 MB + `containerd` 47 MB.

**Headroom: ~1.39 Gi free at steady idle.** Nothing was OOM-killed and no container restarted.
The open question this does *not* answer is what happens under concurrent load, since
`BUZZ_MAX_CONNECTIONS` defaults to 10,000 and `BUZZ_SEND_BUFFER` to 1,000 messages per
connection — that is #39's job.

Startup confirmed the seeding mechanism directly:

```
INFO Database migrations complete
INFO Deployment community ensured  host="buzz-vm.test:3000" community=d97ea868-…
INFO Relay owner bootstrapped      pubkey=38980a43…
```

and `select host from communities` returned exactly one row, `buzz-vm.test:3000` — **the
non-default port is preserved in the community host**, as the normalization rules predict.

## Headline finding — no manual community seeding exists

PRD #2 and issue #19 both assume the deployment hostname must be inserted into the
`communities` table by hand ("records the exact command used"). **There is no such command.**
`buzz-admin` has exactly seven commands — `add-member`, `remove-member`, `list-members`,
`generate-key`, `migrate`, `product-feedback`, `reconcile-channels` — and none of them
touch `communities`.

The relay seeds its own community at startup:

- `crates/buzz-relay/src/main.rs:259-291` calls `db.ensure_configured_community(host)`
- `host` comes from `relay_url_authority(config.relay_url)`
- `ensure_configured_community` is an idempotent `INSERT ... ON CONFLICT (lower(host))`
  (`crates/buzz-db/src/lib.rs:1369`)

So **`RELAY_URL` in `.env` is the single control** that decides which community exists and
therefore which `Host` headers the relay will accept. Issue #19's definition of done needs
rewording — the behaviour it wants to prove is real, the mechanism it names is not.

### The normalization rule that decides accept vs reject

`normalize_host` (`crates/buzz-core/src/tenant.rs:121`) — lowercase, strip a trailing `:443`
**or** `:80`, strip one trailing FQDN dot. `relay_url_authority` produces the byte-identical
shape from `RELAY_URL`, **preserving any non-default port**. Inbound `Host` headers go through
the same function; a mismatch is `BindError::UnmappedHost` and fails closed.

| Where | `RELAY_URL` | `communities.host` becomes | Host header that works |
|---|---|---|---|
| VM, plain HTTP on 3000 | `ws://buzz-vm.local:3000` | `buzz-vm.local:3000` | `buzz-vm.local:3000` |
| VPS, Caddy + TLS | `wss://<domain>` | `<domain>` | `<domain>` (`:443` stripped) |

The trap for #19: leaving the shipped `RELAY_URL=wss://buzz.example.com` while connecting to
`127.0.0.1:3000` seeds `buzz.example.com` but sends `Host: 127.0.0.1:3000` → every connection
rejected. That is PRD #2's headline failure mode, and it is reproducible on the VM by choice
of this one variable.

**`RELAY_URL` is not prefixed `BUZZ_`.** `config.rs:513` reads plain `RELAY_URL`, defaulting to
`ws://localhost:3000`. The fatal-error message in `main.rs:264` calls it `BUZZ_RELAY_URL`, which
is wrong — setting that name has no effect and leaves the localhost default in place.

`buzz-admin` reads the same variable through the same helper (`resolve_admin_tenant`,
`main.rs:439`) and errors out with `RELAY_URL host '<host>' is not mapped to a community` if it
does not match. Two consequences: `add-member` cannot run until the relay has started at least
once and seeded the community, and a `RELAY_URL` mismatch breaks roster management and client
connections identically. One variable, three code paths, deliberately byte-identical.

---

## Provisioning boundary — mirror vs configure

Proposed 2026-08-11, pending ratification of ADR #24. Two layers with a hard line between them:

| Layer | Owns | Runs where |
|---|---|---|
| `build-vps-clone.sh` + cloud-init | The **starting state the provider hands you** — CPU, RAM, disk, NAT forwards, hostname, root SSH access, swap, fallback user | Host, VirtualBox only |
| Ansible under `launchpad/deploy/ansible/` | **Every transformation applied to that state** — Docker, the compose bundle, `.env`, later all hardening | Against both the VM and the VPS, same plays |

Consequences: **Docker does not go into cloud-init.** The build script keeps only the relay port
forward, because that is VirtualBox environment rather than host configuration. And because the
VM's starting state matches the VPS's, the plays that could lock you out of the real server —
#29 (restrict remote root) and #30 (deny-by-default firewall) — get rehearsed destructively here
first.

Scope discipline for now: roles for **docker** and **compose-bundle + `.env`** only. No firewall,
SSH policy, AppArmor, unattended-upgrades or service-identity roles — those are #29–#34 under
PRD #5, and writing them now puts #5's work in front of #2's deadline.

Two implementation notes that keep ownership straight: call upstream `run.sh` from Ansible rather
than reimplementing it with `docker_compose_v2`, and generate secrets **on the target** when
`.env` is absent, which satisfies #22's "generated on the host and appear in no tracked file"
without pre-empting ADR #25's secret-storage question.

## Phase 0 — host prep on the VM

VM is `vps-clone-noble`, currently powered off, 1 vCPU / 2048 MB / 496 M swap, Docker not installed.

1. Boot; capture the pre-Docker baseline for #18 (`free -h`, `swapon --show`, `df -h`, `uname -r`).
2. Add the NAT port forward for the relay — only `2222→22` exists today:
   `VBoxManage modifyvm vps-clone-noble --natpf1 "buzz,tcp,127.0.0.1,3000,,3000"`
3. Install Docker Engine + Compose plugin **from Docker's official apt repo** rather than Ubuntu's
   `docker.io` / `docker-compose-v2`. `deploy/compose/compose.caddy.yml` uses the `!reset` tag,
   which needs **Compose v2.24.4+**. Ubuntu 24.04's archive version sits close to that threshold
   and varies with updates — the point of the official repo is a *known, current* version, not
   that the archive one is certainly broken.
4. Record `docker --version` and `docker compose version` — a DoD line on #18.

**This phase is Ansible's job, not the build script's** — see "Provisioning boundary" below.

## Phase 1 — get the deployment bundle onto the VM

Only `deploy/compose/` is needed: `compose.yml`, `.env.example`, `run.sh`, `compose.caddy.yml`,
`Caddyfile`. The relay itself runs from the prebuilt `ghcr.io/block/buzz` image.

**Do not build from source and do not clone the full repo if avoidable** — ~30 Rust crates on
1 vCPU / 2 GB will thrash or OOM, and #18 explicitly requires the prebuilt image. A sparse or
shallow checkout of `deploy/compose/`, or a plain `scp` of those five files, is enough.

Pin `BUZZ_IMAGE` to `ghcr.io/block/buzz:sha-<7>` rather than `:main` so #18's measurements name
a specific artifact. `:main` is a moving target and makes the report unreproducible.

## Phase 2 — secrets and identities

`run.sh` refuses to start while any `CHANGE_ME` remains (`require_env`, greps the whole file), so
every one of these must exist before first boot. All live only in `deploy/compose/.env`, which is
never committed.

| Variable | What it is | How |
|---|---|---|
| `RELAY_OWNER_PUBKEY` | **Real Nostr identity** that owns the relay. Not `BUZZ_`-prefixed. | `buzz-admin generate-key` |
| `BUZZ_RELAY_PRIVATE_KEY` | Relay's own signing key, separate from the owner | `buzz-admin generate-key` |
| `BUZZ_GIT_HOOK_HMAC_SECRET` | Random 64 hex | `openssl rand -hex 32` |
| `POSTGRES_PASSWORD` | Random | `openssl rand -hex 24` |
| `REDIS_PASSWORD` | Random | `openssl rand -hex 24` |
| `BUZZ_S3_ACCESS_KEY` / `BUZZ_S3_SECRET_KEY` | Become MinIO's root credentials | `openssl rand -hex 16` / `-hex 32` |

`buzz-admin` ships inside the relay image, so keys can be generated before anything else is
running — `connect_db()` is called per-subcommand and `GenerateKey` never calls it
(`main.rs:132-138`), so this works against an empty host with no Postgres:

```bash
docker run --rm --entrypoint /usr/local/bin/buzz-admin ghcr.io/block/buzz:<tag> generate-key
```

It prints both halves of the keypair and tells you to set `BUZZ_PRIVATE_KEY` — ignore that hint
here. For this deployment the public half goes in `RELAY_OWNER_PUBKEY` and a **second, separate**
keypair's secret half goes in `BUZZ_RELAY_PRIVATE_KEY`. Run it twice; do not reuse one keypair
for both.

Two things that hard-fail at startup, both at `main.rs:241` and `main.rs:262`, when
`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` (which `.env.example` ships):

- `BUZZ_RELAY_PRIVATE_KEY` absent → refuses to start. NIP-43 events signed with an ephemeral
  key become unverifiable after restart, so it will not let you.
- `RELAY_URL` with an underivable authority → refuses to start.

**A decision, not a task step:** `RELAY_OWNER_PUBKEY` is a real identity with administrative
control, and whoever holds the matching private key is the relay owner. Whether that is your
personal Nostr identity or a dedicated cohort key that gets escrowed is a cohort call, and it is
easier to settle now than to change after members are rostered.

**No OS-level service accounts are needed at this stage.** Containers run as their images define.
Running Buzz as a non-root declared identity on the host is #31, under PRD #5.

## Phase 3 — bring up and verify

`BUZZ_AUTO_MIGRATE=true` is already set in `.env.example`; the alternative is `buzz-admin migrate`
before first start. Either way a fresh database needs one of them — it is opt-in and off in the
relay's own defaults.

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env            # every CHANGE_ME, plus RELAY_URL and the BUZZ_DOMAIN family
./run.sh config         # renders merged config; catches unset :? vars before pulling
./run.sh start          # up -d --wait
```

Startup order is enforced by `depends_on`: postgres + redis + minio healthy → `minio-init`
one-shot creates the bucket and completes → relay starts. #18 wants peak RSS and swap captured
at four separate points — `docker pull`, migration, first relay start, steady idle — so measure
across this sequence rather than only at the end.

Verification endpoints, all on port 3000 (`router.rs:66-70`).

**Corrected by measurement — an earlier version of this document was wrong here.** It claimed
NIP-11 and the health paths go through tenant binding and therefore need a matching `Host` header.
They do not:

| Request | Seeded `Host` | Bogus `Host` |
|---|---|---|
| `GET /_liveness` | 200 | **200** |
| `GET /health` | 200 | **200** |
| `GET /` with `Accept: application/nostr+json` | 200 | **200** |
| **WebSocket upgrade on `/`** | **101** | **404** |

So `#18`'s bare `curl -fsS http://127.0.0.1:$BUZZ_HTTP_PORT/_liveness` is correct as written — no
`Host` header needed.

**The consequence matters more than the correction.** NIP-11 returning 200 is *not* evidence that
the `Host` → community binding is right; it answers 200 on a misconfigured relay. Any runbook that
verifies binding with NIP-11 will pass while the relay refuses every client. The reliable test is
the WebSocket upgrade:

```bash
K=$(head -c 16 /dev/urandom | base64)
curl -s -i -N --max-time 6 \
  -H "Host: buzz-vm.test:3000" \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' -H "Sec-WebSocket-Key: $K" \
  http://127.0.0.1:3000/ | head -1
# seeded host -> HTTP/1.1 101 Switching Protocols
# bogus host  -> HTTP/1.1 404 Not Found
#                body: "relay: no community is configured for this host"
```

If you would rather check over plain HTTP, the NIP-11 document does differ — a `push` key is
present only when the community resolves, and `supported_extensions` gains `nip-pl`
(`["nip-er","nip-pl"]` resolved vs `["nip-er"]` unresolved; 1321 vs 620 bytes). That is a subtle
tell, not a status code, which is precisely why the upgrade test is the one to write down.

There is also a separate health listener on 8080 carrying only `/_liveness` and `/_readiness`;
that is what the container healthcheck probes, over `/dev/tcp` because the relay image has bash
but no curl or wget.

There is also a separate health listener on 8080 carrying only `/_liveness` and `/_readiness`;
that is what the container healthcheck probes, over `/dev/tcp` because the relay image has bash
but no curl or wget.

Roster management once it is up:

```bash
./run.sh add-member <npub-or-hex> --role member
./run.sh list-members
```

`run.sh help` carries a warning worth keeping: add members **sequentially with `sleep 1`
between calls**, never in parallel, or same-second timestamp collisions corrupt the kind:13534
roster event.

## Phase 4 — what stays out of the VM

TLS cannot be rehearsed here. `compose.caddy.yml` issues via Let's Encrypt, which cannot certify
a name that does not publicly resolve. Serving TLS locally would need a cohort-owned Caddyfile
using `tls internal` under `launchpad/deploy/`, since `deploy/compose/` is upstream-owned and
must not be edited. TLS is verified for real in #22.

---

## Ordering consequence

Phases 0–2 are shared setup for both #18 and #19, and Phase 2's secrets must be stable across
both. But #18 needs a **pristine** VM to measure a genuine first `docker pull`, first migration
and first start. So: snapshot the VM after Phase 0 (Docker installed, nothing pulled), run #18
from there, then continue into #19 on the dirtied guest. Rebuilding via `build-vps-clone.sh`
between them costs a full re-provision and is avoidable if the snapshot is taken.
