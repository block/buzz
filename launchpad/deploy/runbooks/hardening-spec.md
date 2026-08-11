# Hardening specification — Buzz relay, dev VM and production VPS

**Status:** specification only. Nothing here is implemented. This is the document the hardening
Ansible roles are written against, in the same way
[`dev-deployment-SOP.md`](./dev-deployment-SOP.md) is the document `docker` and `compose-bundle`
are written against. If a role does something this document does not describe, one of the two is
wrong and this document changes first.

**Serves:** PRD #5 (#29–#34). **Do not implement ahead of #2** — `../ansible/README.md` is explicit
that writing the hardening roles now puts #5 in front of #2's deadline. Writing the *spec* now is
the thing #5 needs in order to start; writing the *roles* now is the mistake.

**Derived from:** `Research/hardening-linux-servers.md` (generic Ubuntu zero-trust research) applied
to this deployment, plus direct reading of the relay source. Every Buzz-specific finding in Part B
cites the file and line it came from. Part B is the part a generic hardening guide cannot give you,
and it is where the real risk is.

---

## How this fits the mirror / configure boundary

`../AGENTS.md` sets the organising principle, and hardening does not change it:

| Layer | Owns | Hardening's place |
|---|---|---|
| `virtual-box/` | The starting state a provider hands you | **Nothing here.** Its `PermitRootLogin yes` / `PasswordAuthentication yes` is the *insecure starting state the roles must remove*. Leave it alone. |
| `ansible/` | Every transformation applied to that state | **All of it.** One definition, both targets. |

This is the point of the whole arrangement: the dev VM's deliberate insecurity is not a wart, it is
the **test fixture**. A hardening role that cannot turn `PermitRootLogin yes` into `no` on the dev VM
has not been shown to work. Two of these plays can lock you out of a host permanently (#29 remote
root, #30 deny-by-default firewall) and the VM is where you find that out.

One correction to `../ansible/README.md` before the roles start: it contains **two contradictory
rules about where Docker comes from**. The "Decided during that pass" paragraph says the Ubuntu
24.04 archive (`docker.io`, `docker-compose-v2`, `containerd`); Design rule 3 says "Install Docker
from Docker's own apt repo, **not** `docker.io`/`docker-compose-v2`." The SOP (Step 5, and its
verified-findings list) went with the archive and measured Compose 2.40.3, well clear of the 2.24.4
floor. **Keep the archive and delete Design rule 3**, for a hardening reason the paragraph already
gives: Docker patches then ride Ubuntu's security stream, so the `-security` origins in §C7 cover
them. A third-party repo needs its own GPG key on the host *and* its own entry in
`Unattended-Upgrade::Allowed-Origins`, or Docker silently stops receiving automatic patches — the
exact failure mode that looks fine for months.

---

## Part A — The parity model

Your instinct is right, and it is worth stating precisely because it is the thing that makes this
tractable: **one baseline, one role set, one verification suite; dev and prod differ only in the
value of a short list of variables.** The reason this matters is not tidiness. It is that a control
which is relaxed in dev is a control that has never been executed before it runs on prod, and the
two plays that can lock you out are exactly the ones you least want to run for the first time
against the real server.

So the model is: **the property is identical, the mechanism may differ.** "No SSH reachable from an
untrusted network" is the property. On the VM it holds because VirtualBox forwards `2222` from
`127.0.0.1` only; on the VPS it holds because there is no public listener at all. Same property,
different mechanism, and both are checked by the same assertion.

### A.1 What is genuinely identical

Everything in Part B and Part C, with no exceptions and no dev carve-outs:

- SSH policy — no root, no passwords, certificate or key auth only, and the removal of
  `01-dev-parity.conf`
- Deny-by-default ingress **and egress** firewall, with the same allowlist
- Kernel sysctl set and module blacklist
- systemd sandboxing on Docker and any host service
- Docker daemon configuration and the container network split
- Secret generation on the target, `.env` at `0600`, never committed
- Image pinning by digest and signature verification
- `unattended-upgrades`, `needrestart`, reboot detection
- Log shipping, `auditd` rules, `sudo` I/O logging
- The full verification suite in Part D — dev must pass every assertion prod passes

### A.2 The nine variables that differ

| Variable | Dev VM | Production VPS |
|---|---|---|
| `buzz_domain` | `buzz-vm.test` | the cohort's real domain |
| `buzz_relay_url` | `wss://buzz-vm.test` | `wss://<real domain>` |
| `buzz_tls_mode` | `internal` (Caddy's local CA) | `acme` (Let's Encrypt) |
| `buzz_acme_email` | unset | the cohort's ops address |
| `buzz_admin_host` | `admin.buzz-vm.test` | **unset** — see §B1 |
| `buzz_auto_migrate` | `true` | `false` — see §B6 |
| `buzz_max_connections` | measured VM ceiling | measured VPS ceiling — see §B9 |
| `buzz_backup_target` | a local directory | object storage with Object Lock |
| `buzz_log_target` | local journal only | remote collector |

Nine values. That is the whole difference. Note `buzz_admin_host` and `buzz_auto_migrate` are the
two where **dev is deliberately more permissive**, and both are called out in Part B with the reason;
everything else is the same setting pointed at a different place.

### A.3 The one change to the SOP that buys the most parity

**Run Caddy in dev too, with `tls internal`.**

Today the SOP's dev path publishes the relay directly on port 3000 over plaintext HTTP
(`compose.yml` `ports: "${BUZZ_HTTP_PORT:-3000}:3000"`), and Step 12.1 has to warn the reader to
type `ws://` because "production will use `wss://`; this VM does not." That single sentence is the
parity model leaking: it means the dev environment exercises a *different code path*, a different
port, a different Compose file set, and a different client configuration than production.

`compose.caddy.yml` already exists upstream and already does the right thing — it adds Caddy and
sets `relay: ports: !reset []`, which **removes the relay's published port entirely**. Turning it on
in dev via `BUZZ_COMPOSE_TLS=true` with a cohort-owned Caddyfile using `tls internal`:

- makes the dev and prod Compose file sets identical
- makes both `wss://`, on 443, through a reverse proxy
- deletes the port-3000 exposure from §B2 in dev as well as prod
- exercises the Caddy config, the header set, and the admin vhost split in dev
- removes the `ws://` special case from Step 12.1

`../AGENTS.md` already anticipates this — "Cohort-owned overrides (for example a `tls internal`
Caddyfile) live here, not there." This spec assumes it. The cost is one extra port forward
(`443`) on the VM and trusting Caddy's local CA root on your own machine.

---

## Part B — Buzz-specific hardening

These are the findings that come from this deployment rather than from Ubuntu generally. They are
ordered by severity. Each states the finding with a source reference, why it matters on a public
host, the required control, and how to verify it.

### B1 — The admin API's only credential is a `Host` header  ⚠️ highest severity

**Finding.** `crates/buzz-relay/src/api/admin/auth.rs:16-33`. `authorize()` grants access when the
request's `Host` header equals `BUZZ_ADMIN_HOST`, and the `Origin` header either matches or **is
absent**:

```rust
if headers.get(header::ORIGIN).is_some_and(|origin| { ... !origin_matches_host(...) }) {
    return Err(ApiError::forbidden());
}
Ok(())
```

There is no identity check, no token, no Nostr auth, no relay-membership check. `curl` sends no
`Origin`, so the `Origin` branch is skipped entirely — it is a browser CSRF guard, not
authentication.

The SOP demonstrates this without flagging it. Step 9.6 runs
`curl -H "Host: admin.buzz-vm.test:3000" .../api/admin/v1/reports` with no credentials and records
`[] <- 200` as the expected successful result. That is correct for a loopback-bound VM. On a public
host, the same request from anywhere on the internet returns the same `200`.

**Why it matters in prod.** The routes are `/reports`, `/reports/{id}`, `/feedback`, `/feedback/{id}`
and `/feedback/{id}/attachments/{sha256}` (`crates/buzz-relay/src/api/admin/mod.rs:28-40`) — user-submitted
moderation reports and product feedback, including file attachments. That is unauthenticated
disclosure of exactly the content a closed community would consider sensitive, and the only thing
standing in front of it is guessing a hostname.

**Required control — prod leaves `BUZZ_ADMIN_HOST` unset.** `crates/buzz-relay/src/config.rs:910-937`
returns `None` for the whole admin config when the variable is empty, and `router.rs:60` only mounts
`/api/admin/v1` when it is `Some`. So unsetting it removes the surface entirely. Two details that
make this clean:

- `BUZZ_ADMIN_WEB_DIR` is read **inside** the `Some(host)` branch (`config.rs:923`), so with the host
  unset the web-dir variable is ignored and does **not** cause a startup error. You can leave it in
  `.env`. This refines `../ansible/README.md`'s note that "`BUZZ_ADMIN_WEB_DIR` is validated at
  relay startup against `index.html`" — true, but only when `BUZZ_ADMIN_HOST` is also set.
- To use the dashboard, set the variable and restart inside a maintenance window, reaching it over
  the mesh, then unset it. The dashboard is read-only (SOP Step 9), so this costs nothing
  operationally.

**If the cohort wants it always on**, all three of these, not any one alone:

1. The admin hostname must not resolve in public DNS — mesh-internal only.
2. A separate Caddy site block for it with real authentication (`forward_auth` to the IdP, or client
   certificate auth). Never a wildcard site block that proxies any `Host` to the relay.
3. `ufw`/mesh ACL restricting the admin vhost's listener to the mesh interface.

**Verify.** From outside the mesh, against prod:
```bash
curl -s -o /dev/null -w "%{http_code}\n" -H "Host: admin.<domain>" https://<domain>/api/admin/v1/reports
```
Must be `404` (surface absent) or `403`. A `200` is a finding. Run the same assertion in dev with the
dev admin host and expect `200` there — that difference is deliberate and is one of the nine
variables.

### B2 — `ports: 3000:3000` bypasses `ufw`, and prod bring-up does not turn Caddy on

**Finding.** Three facts that combine badly:

1. `deploy/compose/compose.yml` publishes the relay: `ports: - "${BUZZ_HTTP_PORT:-3000}:3000"`.
2. `deploy/compose/run.sh` gates Caddy behind `BUZZ_COMPOSE_TLS`, defaulting to **false**, and
   `.env.example` does not set it — it only sets `BUZZ_HTTP_PORT=3000`, `CADDY_HTTP_PORT=80`,
   `CADDY_HTTPS_PORT=443` with a comment reading "Base compose publishes the relay directly on
   `BUZZ_HTTP_PORT`."
3. The SOP's own restart command, which `../ansible/README.md` says the `compose-bundle` role will
   encode, is `docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait` —
   **`compose.caddy.yml` is not in that list.**

So `./run.sh start` on the VPS, or the role as currently specified, yields a **plaintext relay
published on port 3000 to the internet**, with no TLS and no reverse proxy.

And published Docker ports are inserted into the `DOCKER` iptables chain, which is evaluated
*before* `ufw`'s rules. **`ufw deny 3000` will not close it.** This is the Docker/`ufw` trap from
`Research/hardening-linux-servers.md` §3.20, and this deployment walks straight into it. A reader who
hardens the firewall, tests `ufw status`, and sees `3000 DENY` will believe the port is closed while
it is serving the internet.

**Required control.**

1. **`BUZZ_COMPOSE_TLS=true` on both targets**, and the role's Compose invocation must be
   `-f compose.yml -f compose.caddy.yml -f compose.cohort.yml`. `compose.caddy.yml`'s
   `relay: ports: !reset []` is the *only* thing that closes 3000 — it is not optional hardening, it
   is the control.
2. **Belt and braces:** in `compose.cohort.yml`, bind any port that must stay published to loopback
   —`"127.0.0.1:3000:3000"` rather than `"3000:3000"`. A loopback-bound published port is not
   reachable off-host regardless of what iptables does.
3. **Never rely on `ufw` alone for a container port.** Host policy for containers goes in the
   `DOCKER-USER` chain, with the ordering fix from `Research/hardening-linux-servers.md` §1.2
   (conntrack rule first, `DROP` appended last — an unconditional `-I DOCKER-USER -j DROP` blocks
   return traffic and kills all container networking).
4. Add an assertion to the verification suite that fails if `docker compose ps --format json` shows
   any published port bound to `0.0.0.0` or `::`.

**Verify.** From a machine that is not the host:
```bash
nmap -Pn -p 3000,5432,6379,8080,9000,9001,9090,9102 <host>
```
Every one must be `closed` or `filtered`. Do this from off-host — `ss -tlnp` on the host will show
Docker's proxy listening and tells you nothing about reachability.

### B3 — `compose.dev.yml` puts Adminer and every datastore on the network

**Finding.** `deploy/compose/compose.dev.yml` publishes Postgres `5432`, Redis `6379`, MinIO
`9000`/`9001`, **Adminer on `8082`**, and Prometheus on `9090`. It is gated by `BUZZ_COMPOSE_DEV`,
default false.

`BUZZ_COMPOSE_DEV=true` on the VPS means a database administration web GUI on the public internet,
in front of a Postgres whose password is in the same `.env`.

**Required control.** A hard assertion in the role, failing the play rather than warning:
`BUZZ_COMPOSE_DEV` must be absent or `false` whenever the target is not the dev VM. Combine with
B2's off-host port scan, which catches it empirically even if the variable check is bypassed.

Note this is one place where dev and prod genuinely diverge in *file set* rather than variable value
— and it is fine, because the divergence adds a debugging surface rather than relaxing a control.
Keep it that way: nothing in `compose.dev.yml` should ever become load-bearing.

### B4 — The relay holds MinIO root credentials

**Finding.** `deploy/compose/compose.yml`: `MINIO_ROOT_USER: ${BUZZ_S3_ACCESS_KEY}` and
`MINIO_ROOT_PASSWORD: ${BUZZ_S3_SECRET_KEY}` — the same pair the relay gets as
`BUZZ_S3_ACCESS_KEY`/`BUZZ_S3_SECRET_KEY`. The relay is running as MinIO's root account.

**Why it matters.** A compromised relay does not just read media, it administers the object store:
create and delete buckets, and set bucket policy to public. The `minio-init` container correctly runs
`mc anonymous set none` on the bucket, but root can undo that.

**Required control.** Least privilege, per `Research/hardening-linux-servers.md` §3 identity
guidance: generate a MinIO **service account** scoped to `s3:GetObject`/`PutObject`/`DeleteObject` on
`buzz-media` only, hand *that* to the relay, and keep root credentials out of the relay's
environment. This needs a cohort-owned extension of the `minio-init` step (not an edit to
`compose.yml`, per hard rule 1) plus separate `.env` keys for root and service credentials. Assert
`mc anonymous get local/buzz-media` returns `none` in the verification suite.

### B5 — `buzz-net` is a plain bridge, so every container has unrestricted egress

**Finding.** `deploy/compose/compose.yml` ends with:
```yaml
networks:
  buzz-net:
    driver: bridge
```
Not `internal: true`. Postgres, Redis and MinIO all sit on it with full outbound internet access, and
nothing needs it.

**Why it matters.** This is the concrete instance of the egress gap
(`Research/hardening-linux-servers.md` §3.2). Every post-exploitation step is outbound: a compromised
Postgres can stream the community's entire message history anywhere, and no ingress rule sees it.

**Required control.** Split the network in `compose.cohort.yml`:

- `buzz-internal` with `internal: true` — postgres, redis, minio, minio-init
- `buzz-net` (existing bridge) — relay and caddy
- relay on **both**

Because the base file already attaches each service to `buzz-net`, the override must *replace* rather
than merge that list — Compose's `!override` tag is the tool, and 2.40.3 supports it (the same
mechanism `compose.caddy.yml` uses for `!reset`). **Verify the exact tag form on the pinned Compose
version before committing the role**; this is the one snippet in this document whose syntax has not
been executed here:

```yaml
services:
  postgres:
    networks: !override [buzz-internal]
  redis:
    networks: !override [buzz-internal]
  minio:
    networks: !override [buzz-internal]
  minio-init:
    networks: !override [buzz-internal]
  relay:
    networks: !override [buzz-net, buzz-internal]

networks:
  buzz-internal:
    internal: true
```

Also: `"icc": false` in `daemon.json` (§C10) so containers cannot reach each other except on networks
you defined, and block `169.254.169.254` from every container network so a relay-side SSRF cannot
reach cloud instance metadata (`Research/hardening-linux-servers.md` §3.4 — the standard
SSRF-to-cloud-credentials pivot).

**Verify.** `docker compose exec postgres sh -c 'timeout 3 getent hosts example.com'` must fail, and
the relay's own egress must be confined to the §C4 allowlist.

### B6 — `BUZZ_AUTO_MIGRATE=true` migrates prod's schema on container start

**Finding.** `deploy/compose/.env.example:19` ships `BUZZ_AUTO_MIGRATE=true`, and the SOP's Step 7.3
explicitly tells the reader to leave it alone: "the relay creates its database tables on first
start." But `compose.yml` itself defaults it to **false**
(`BUZZ_AUTO_MIGRATE: ${BUZZ_AUTO_MIGRATE:-false}`), which tells you upstream considers `false` the
production value.

**Why it matters.** With it true, `./run.sh upgrade` — which does `compose pull` then `up` — can apply
schema migrations to the production database as a side effect of pulling a newer image, with no
backup gate, no dry run, and no rollback. Combined with `restart: unless-stopped`, an unattended
restart can do it too.

**Required control.** `buzz_auto_migrate: false` in prod. Migrations become a deliberate step in the
upgrade runbook: verified `pg_dump` first, then run migrations explicitly, then start. Keep `true` in
dev — that is one of the nine variables, and it is the correct value for a throwaway VM.

### B7 — Every secret is in the container environment

**Finding.** `compose.yml` uses `env_file: - .env` plus an `environment:` block, so
`POSTGRES_PASSWORD`, `REDIS_PASSWORD`, `BUZZ_S3_SECRET_KEY`, `BUZZ_RELAY_PRIVATE_KEY` and
`BUZZ_GIT_HOOK_HMAC_SECRET` all land in the relay process's environment. `DATABASE_URL` and
`REDIS_URL` embed passwords directly.

This is the anti-pattern from `Research/hardening-linux-servers.md` §3.5 — environment variables are
readable via `/proc/<pid>/environ`, appear in `docker inspect`, and are captured by core dumps and
crash reporters. Here it is structural: it comes from upstream's Compose file, which hard rule 1
forbids editing.

**Required control — what is achievable now:**

- `.env` at `0600`, owned by root. The SOP already does this (Step 7) — keep it and assert it.
- Generate on the target, never templated from the control node, never committed (`../AGENTS.md`
  rule 3). Already the rule; the hardening role must not weaken it.
- **Treat the `docker` group as root-equivalent and document it.** Anyone in it can read every
  secret and mount the host filesystem. The deploy user being in `docker` is not a privilege
  reduction, and the SOP's all-root workflow should not be replaced with a non-root user *while
  implying* it is safer.
- `"no-new-privileges": true` in `daemon.json`, and `fs.suid_dumpable=0` + `kernel.core_pattern=|/bin/false`
  (§C5) so a crash cannot spill the environment to disk.
- Add `docker inspect` to the list of secret-bearing commands in the runbook, so nobody pastes its
  output into an issue.

**What needs upstream work, not a local hack:** Docker secrets or `*_FILE` environment support in the
relay, so secrets arrive as files rather than variables. Worth an upstream issue against `block/buzz`
rather than a fork-local patch that becomes a merge conflict every sync.

### B8 — The relay's private key is unrecoverable and unrotatable

**Finding.** SOP Step 7.5: the relay refuses to start with a temporary key "because messages signed
with one become unverifiable after a restart." So `BUZZ_RELAY_PRIVATE_KEY` is not a rotatable
credential — losing it breaks signature verification for every message the relay ever signed, and
changing it has the same effect.

**Why it matters.** This is a different class of secret from a database password. A lost Postgres
password is an afternoon; a lost relay key is a community-wide, permanent integrity break. It cannot
be treated as one line in `.env`.

**Required control.** Escrow it separately from the rest of `.env`: encrypted, in at least two
locations, with a documented and *tested* recovery procedure, and separate from the backup
credentials so one compromise does not take both. Same treatment for the owner secret key from Step
7.1, which is the administrator's login. `run.sh backup-hint` already names both — turn that print
statement into an actual automated, verified backup (§E).

Also worth writing down explicitly, because it is not obvious: there is **no key rotation path**. If
the key is suspected compromised, the answer is a new community, not a new key. That belongs in the
incident-response runbook.

### B9 — Rate limits are per-identity; nothing limits unauthenticated load

**Finding.** `crates/buzz-relay/src/config.rs:319-347` exposes a good set of limits —
`BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN`, `..._API_CALLS_PER_MIN`, `..._WS_EVENTS_PER_SEC`, and agent
standard/elevated/platform tiers. All of them key off an **authenticated identity**, so they shape
behaviour *after* auth. `BUZZ_MAX_CONNECTIONS` (`config.rs:535`) defaults to 10,000 per the SOP.

`.env.example` sets none of them.

**Why it matters.** 10,000 connections on a 1.9 GiB VPS that already idles at 563 MB, where the SOP's
own honesty section says "behaviour under load … is unmeasured and could look very different," and
each connection buffers up to 1,000 messages. Availability is a stated objective in the research doc
with no controls behind it (`Research/hardening-linux-servers.md` §3.8); this is where that bites.

**Required control.**

1. Set `BUZZ_MAX_CONNECTIONS` to a measured ceiling, not the default. Measure it on the VM — this is
   exactly what #18's capacity work is for, and the VM has RAM parity even though it lacks disk
   parity.
2. Set the rate-limit variables explicitly rather than inheriting defaults, so the values are
   reviewable in `.env`.
3. Connection-level limits at the proxy, in front of auth. **Caddy v2 has no built-in rate
   limiter** — this needs the `caddy-ratelimit` plugin, which means a custom Caddy image, or a CDN in
   front. Flag the choice rather than assuming; a custom image also means an image build the cohort
   now owns and must patch.
4. Timeouts and body caps in the Caddyfile (§B10).
5. systemd `MemoryMax=`/`TasksMax=`/`CPUQuota=` on `docker.service` (§C6) so a runaway container
   cannot take the host's SSH down with it — on a 1.9 GiB box, OOM is the realistic availability
   failure, not a flood.

### B10 — The Caddyfile is four lines

**Finding.** `deploy/compose/Caddyfile` in full:
```
{$BUZZ_DOMAIN} {
  encode zstd gzip
  reverse_proxy relay:3000
}
```

Caddy does add HSTS automatically on HTTPS sites, and that is the only security header present.
There are no timeouts, no request-size limits, no header policy, and — importantly — **no site block
for the admin host**, so it is undefined how `admin.<domain>` reaches the relay in production. If
someone solves that with a wildcard site block, they have handed B1 to the internet.

**Required control.** A cohort-owned Caddyfile in `launchpad/deploy/` (hard rule 1 — overrides live
here, not in `deploy/compose/`), with:

- an explicit site block per hostname, **no wildcards**
- the header set from `Research/hardening-linux-servers.md` §3.10 — CSP with `frame-ancestors 'none'`
  and `base-uri 'none'`, `Referrer-Policy`, `Permissions-Policy`, the three `Cross-Origin-*` headers,
  `X-Content-Type-Options`. Note CSP `frame-ancestors` supersedes `X-Frame-Options`; keep the latter
  only for legacy clients
- `request_body { max_size 10MB }` and read/write timeouts
- WebSocket upgrade preserved on `/` — the relay serves a page to browsers, a NIP-11 document to
  Nostr clients, and a WebSocket to apps on the *same* address (SOP Step 10), so a naive header or
  routing rule can break one surface while the others still work. Test all three after any Caddyfile
  change
- `tls internal` in dev, ACME in prod — the `buzz_tls_mode` variable from §A.2
- a decision on which paths are exposed at all: git smart HTTP, `/hooks/{id}` webhooks, and Blossom
  media are separate surfaces. If the community does not use git over HTTP, do not proxy it

**Verify.** `testssl.sh --severity HIGH https://<domain>` plus a check that all three `/` behaviours
still work: browser `Accept: text/html` → `200 text/html`, Nostr `Accept: application/nostr+json` →
NIP-11 JSON, and a WebSocket upgrade → `101`.

### B11 — Health, metrics and NIP-11 are not tenant-gated

**Finding.** From the SOP's verified findings and Step 8.2: `/health`, `/_liveness` and the relay
information document at `/` return `200` for **any** `Host`. Only the WebSocket upgrade is
community-gated. The commit message for the SOP records the same thing.

`compose.yml` sets `BUZZ_HEALTH_PORT: "8080"` and `BUZZ_METRICS_PORT: "9102"` but publishes neither —
correct, and the verification suite should assert it stays that way.

**Why it matters.** Low severity, but for a closed community it means anyone can confirm the relay
exists and read its NIP-11 metadata, including software version. That is fingerprinting for
CVE-matching.

**Required control.** Restrict `/health` and `/_liveness` to the mesh/loopback in the Caddyfile.
Decide deliberately whether NIP-11 at `/` should be public — it probably must be, for clients to
discover capabilities, so treat the version disclosure as accepted risk and write it down rather than
discovering it in a scan later. If prod metrics are wanted, scrape over the mesh; never publish 9102.

### B12 — Images are pinned by mutable tag, not digest, and signatures are unverified

**Finding.** `scripts/resolve-image-tag.sh` and SOP Step 6.1 resolve `BUZZ_IMAGE` to
`ghcr.io/block/buzz:sha-<7 chars>` — pinned to the upstream commit the fork is synced to, which is
genuinely good practice and solves the "`main` moves" problem the SOP calls out.

But a `sha-abc1234` **tag** is still mutable: it can be re-pushed to point at a different manifest.
And nothing verifies a signature.

**Required control.**

1. Resolve one step further, to an immutable digest: `ghcr.io/block/buzz@sha256:<64 hex>`. The
   existing helper already fetches the manifest to check for a `200` (SOP Step 6.1) — capture the
   `Docker-Content-Digest` response header from that same call and pin to it. Small change, closes
   the mutability gap.
2. Record the digest in `UPSTREAM_COMMIT` alongside the commit, so a reviewer can tell exactly what
   ran.
3. Check whether `block/buzz` publishes Cosign attestations; if it does, `cosign verify` in the role
   and fail closed. If it does not, that is an upstream ask.
4. Relevant deadline from the research: **Docker Content Trust and Notary v1 shut down 8 December
   2026.** If any cohort tooling sets `DOCKER_CONTENT_TRUST=1`, it breaks then. Sigstore/Cosign is
   the replacement.
5. Scan the pinned image (Trivy/Grype) with a **failing** threshold in CI, not a report nobody reads.

### B13 — The membership roster has a correctness trap that is also an availability one

**Finding.** SOP Step 11 and `run.sh help`: adding members in a loop without `sleep 1`, or in
parallel, "can corrupt the membership record because entries share a timestamp" in the kind:13534
roster event.

**Why it matters here.** It is documented as an operational footgun, but it is also the closest thing
to a data-integrity bug in the deploy path — and the roster is the access control list. A corrupted
roster is either a lockout or an unintended grant.

**Required control.** The Ansible role that manages membership must serialise adds with a delay,
never `loop` with `async`, and must verify with `./run.sh list-members` after each batch. Back the
roster up (it lives in Postgres, so §E covers it) and add a "roster matches intended state"
assertion to the verification suite. Also note SOP Step 11's honesty flag: whether an *unapproved*
key is actually refused is untested — a `101` upgrade proves the community matched, not that
membership was enforced, because auth happens after the upgrade. Rehearse that on the VM before
relying on `BUZZ_REQUIRE_RELAY_MEMBERSHIP` in prod.

---

## Part C — Host baseline

Same on both targets. Mapped to the existing issue numbers where `../ansible/README.md` names them.

### C1 — Access path (#29)

No SSH listener reachable from an untrusted network, on either target.

- **Dev:** already true via VirtualBox NAT forwarding `127.0.0.1:2222` only. Keep it; do not add a
  bridged adapter.
- **Prod:** SSM Session Manager, or a WireGuard mesh (Tailscale/Netbird) with SSO and ACLs, or
  Teleport if the audit trail matters more than the setup cost. Then `ufw deny 22`.
- **Break-glass is mandatory and must be built at the same time**, not later. The moment #29 and #30
  land you have created a total-lockout mode. Cloud serial console access, a sealed credential
  outside the repo and outside the vault the deploy uses, and an alert that fires when it is used.
  Rehearse the lockout on the VM — that is what `../ansible/README.md` means by "worth more than the
  automation itself."

### C2 — SSH policy (#29), and the ordering trap that will silently defeat it

**This is the single most likely way the hardening role passes review and does nothing.**

The dev VM's cloud-init writes `/etc/ssh/sshd_config.d/01-dev-parity.conf` containing
`PermitRootLogin yes` / `PasswordAuthentication yes`. The SOP explains why at Step 3.5: "sshd uses
the FIRST value it finds for a setting … This file must sort BEFORE that one to win, hence the `01-`
prefix."

That reasoning cuts both ways. A hardening role that drops
`/etc/ssh/sshd_config.d/99-hardening.conf` with `PermitRootLogin no` **will change nothing** —
`01-dev-parity.conf` is read first and first value wins. The play reports `changed`, the file exists,
and root login over password still works.

So the role must, in this order:

1. `file: path=/etc/ssh/sshd_config.d/01-dev-parity.conf state=absent`
2. write its own policy file
3. `sshd -t` to validate before reloading
4. reload, and **verify with `sshd -T`, never with "the file is present"**

Policy: `PermitRootLogin no`, `PasswordAuthentication no`, `KbdInteractiveAuthentication no`,
`PubkeyAuthentication yes`, `MaxAuthTries 3`, `MaxStartups 10:30:60`, `LoginGraceTime 30`,
`PermitEmptyPasswords no`, `AllowTcpForwarding no`, `X11Forwarding no`, `AllowUsers <deploy user>`,
modern `Ciphers`/`MACs`/`KexAlgorithms` only.

Beyond keys, per `Research/hardening-linux-servers.md` Part 2: `authorized_keys` is a standing bearer
credential and the opposite of continuous verification. Target state is an SSH certificate authority
issuing 5–15 minute certs (`TrustedUserCAKeys`, `AuthorizedPrincipalsFile`,
`AuthorizedKeysFile none`), so revocation is "stop issuing" rather than "edit a file on every host."
Rehearse on the VM.

Also remove the dev VM's `dev` user `sudo: ALL=(ALL) NOPASSWD:ALL`. Prod's deploy user gets a narrow
command allowlist, `Defaults use_pty`, and `Defaults log_output` with `iolog_dir` — which is the
concrete version of the "SSH session recording" the research doc waves at.

**Verify.** `sshd -T | grep -E 'permitrootlogin|passwordauthentication|kbdinteractive'` must show
`no`, `no`, `no`. Assert this on **both** targets — the dev VM is where it must be proven, because it
is the one that starts out permissive.

### C3 — Ingress firewall (#30)

Default deny inbound; allow only 443 (and 80 for ACME redirect, prod only) plus the mesh interface.
`ufw` for the host, `DOCKER-USER` for containers, with §B2's ordering fix. Remember `ufw` does not
govern published container ports — that is what B2 is about.

Prove it from off-host with `nmap`, never from the host with `ss`.

### C4 — Egress firewall (#30) — absent from the research doc's checklist, and the highest-value addition

Default-deny `OUTPUT`, plus §B5's internal network for the datastores. The allowlist is small and
knowable here, which is what makes this practical:

- Ubuntu archive + security mirrors (via an explicit proxy, since mirror IPs move)
- `ghcr.io` for image pulls
- Let's Encrypt ACME endpoints and OCSP (prod)
- the mesh control plane
- the log collector and backup target
- DNS to your resolver only

Everything else denied and **logged**. Denied egress is the highest-signal detection available on
this host. Discover the allowlist's gaps on the VM, where being wrong costs nothing — that is the
whole argument for identical dev/prod egress policy.

### C5 — Kernel parameters (#31)

The full set from `Research/hardening-linux-servers.md` §3.7, not the five in its own example. In
particular `kernel.kptr_restrict=2`, `dmesg_restrict=1`, `yama.ptrace_scope=1`,
`unprivileged_bpf_disabled=1`, `bpf_jit_harden=2`, `kexec_load_disabled=1`, `sysrq=0`,
`perf_event_paranoid=3`, `core_pattern=|/bin/false`, the four `fs.protected_*`, and the fuller
`net.ipv4.conf.all.*` set. Plus the module blacklist with `install <mod> /bin/false`.

**One conflict specific to this deployment:** `net.ipv4.ip_forward=0` — which the research doc
recommends twice — **breaks Docker**, which requires forwarding and will re-enable it. Do not script
it here. Comment the exception in the role so the next reader does not "fix" it.

### C6 — systemd sandboxing (#31)

Absent from the research doc entirely and the highest value per line on Ubuntu. There are few host
services on this box, which makes it cheap: `docker.service`, `ssh`, `chrony`, the log agent.

For `docker.service` the useful directives are the resource caps rather than the filesystem ones
(Docker needs broad access by design): `MemoryMax=`, `TasksMax=`, `CPUQuota=`, so a container OOM
cannot take SSH down with it — the realistic availability failure on a 1.9 GiB host.

Gate it in CI: `systemd-analyze security <unit>` returns a numeric exposure score. Assert a maximum
rather than eyeballing it.

### C7 — Patching (#32)

`unattended-upgrades` on `${distro_id}:${distro_codename}-security` (**not** `universe` — the
research doc has this backwards at its line 60; universe gets no guaranteed security updates), plus
the ESM origins if the cohort has Ubuntu Pro. `Automatic-Reboot "false"` — reboots are orchestrated,
not surprise.

Then the parts that are usually missed: `needrestart -b` to catch services still running against
deleted libraries, `/var/run/reboot-required` detection, and a written patch SLA by severity. If
Design rule 3 survives and Docker comes from a third-party repo, that origin needs its own
`Allowed-Origins` entry or Docker stops being patched silently — see the note at the top.

### C8 — Logging and audit (#33)

Ship first, harden locally second: an attacker with root deletes `/var/log`, so the control is
getting events off the host quickly. Remote collector in prod, local journal in dev
(`buzz_log_target`). journald Forward Secure Sealing for local tamper-evidence. `auditd` from a
maintained ruleset with `-e 2`, and `space_left_action` set deliberately — the default `SUSPEND` can
halt the host. `sudo` I/O logging per §C2.

Buzz-specific: `RUST_LOG` is set to `info` across `buzz_relay,buzz_db,buzz_auth,buzz_pubsub,tower_http`
in `.env.example:21`. Confirm the relay's logs at `info` do not contain message content, pubkeys
beyond what is needed, or auth tokens before shipping them anywhere — a log pipeline that exfiltrates
what the relay is supposed to protect is worse than no pipeline.

### C9 — Time (#33)

`chrony` with **NTS** (authenticated NTP), not plain NTP. Log timestamps are only as trustworthy as
the clock, and unauthenticated NTP is attacker-movable. Matters more than it looks: the roster's
timestamp collision issue (§B13) means clock behaviour has correctness consequences here, not just
forensic ones.

### C10 — Docker daemon (#34)

`/etc/docker/daemon.json` — and **do not use the research doc's example**, which is broken. Its
`disable-legacy-registry` key was removed in Docker 17.12 and makes `dockerd` refuse to start, and
its `"ip-forward-no-drop": true` is the documented opt-out from Docker 28's default-drop of
unpublished ports, i.e. it disables the protection the same document recommends elsewhere. Both are
corrected in `Research/hardening-linux-servers-gap-analysis.md` §1.1.

Use instead:
```json
{
  "no-new-privileges": true,
  "icc": false,
  "live-restore": true,
  "log-driver": "journald",
  "userland-proxy": false,
  "default-ulimits": { "nofile": { "Name": "nofile", "Hard": 4096, "Soft": 1024 } }
}
```
Run `dockerd --validate` and gate the restart on it, so a bad key cannot take Docker out on the VPS.

On `userns-remap`: valuable, but it breaks bind-mount ownership and cannot be enabled on a host with
existing containers without a migration. The SOP's `compose.cohort.yml` bind-mounts `/opt/buzz/web`
and `/opt/buzz/admin-web` read-only, so this needs rehearsing on the VM before prod. Not a drop-in
flag.

Rootless Docker is worth *considering* and probably rejecting: it complicates binding 80/443, which
Caddy needs. Prefer the network split (§B5), `no-new-privileges`, and treating the `docker` group as
root-equivalent (§B7).

---

## Part D — Verification suite

One script, run against both targets, every control asserted. This is the piece that makes the parity
model real rather than aspirational — if it passes on dev and prod alike, the baseline is genuinely
shared.

Rules: it must be **runnable by a reviewer**, it must **fail** rather than warn, and the network
assertions must run **from off-host** because on-host checks lie about container ports.

| # | Assertion | Command | Source |
|---|---|---|---|
| 1 | No root, no password SSH | `sshd -T \| grep -E 'permitrootlogin\|passwordauthentication'` | §C2 |
| 2 | `01-dev-parity.conf` gone | `test ! -f /etc/ssh/sshd_config.d/01-dev-parity.conf` | §C2 |
| 3 | No unexpected open ports | `nmap -Pn -p- <host>` from off-host | §B2 |
| 4 | No `0.0.0.0` published ports | parse `docker compose ps --format json` | §B2 |
| 5 | Admin API absent in prod | `curl -H "Host: admin.<domain>" .../api/admin/v1/reports` → `404`/`403` | §B1 |
| 6 | `BUZZ_COMPOSE_DEV` off in prod | grep `.env` + assertion 3 | §B3 |
| 7 | Datastores have no egress | `docker compose exec postgres getent hosts example.com` fails | §B5 |
| 8 | Media bucket not public | `mc anonymous get local/buzz-media` → `none` | §B4 |
| 9 | Egress default-deny | `iptables -S OUTPUT \| head -1` → `-P OUTPUT DROP` | §C4 |
| 10 | Auto-migrate off in prod | grep `.env` | §B6 |
| 11 | `.env` is `0600` root | `stat -c '%a %U' .env` → `600 root` | §B7 |
| 12 | Image pinned by digest | grep `BUZZ_IMAGE` for `@sha256:` | §B12 |
| 13 | systemd exposure score | `systemd-analyze security docker.service` under threshold | §C6 |
| 14 | Sysctl set applied | `sysctl -a` diff against expected | §C5 |
| 15 | Unattended-upgrades origins | grep `Allowed-Origins`, assert no `universe` | §C7 |
| 16 | TLS grade | `testssl.sh --severity HIGH` | §B10 |
| 17 | All three `/` behaviours work | browser HTML, NIP-11 JSON, WebSocket `101` | §B10 |
| 18 | Wrong `Host` still refused | SOP Step 8.2's negative test | SOP |
| 19 | Roster matches intent | `./run.sh list-members` diff | §B13 |
| 20 | Restore actually works | timed restore drill, recorded numbers | §E |

Assertions 17 and 18 are the ones to run after *every* Caddyfile or firewall change — the relay
serves three different things on one address, and it is easy to fix one while breaking another
without noticing.

Existing off-the-shelf gates worth adding once the above passes: `lynis audit system`,
OpenSCAP against the Ubuntu CIS profile, and Trivy on the pinned image with a failing threshold.

---

## Part E — Backup and recovery

`run.sh backup-hint` prints a correct checklist and automates nothing. Turn it into a role.

**What must be backed up, and why each one is non-obvious:**

| Item | Why |
|---|---|
| `BUZZ_RELAY_PRIVATE_KEY` | Loss is a permanent, community-wide signature break, not an inconvenience (§B8) |
| Owner secret key | The administrator's login; generated in SOP 7.1 and stored only in a password manager |
| Rest of `.env` | DB/Redis/S3 secrets, `BUZZ_GIT_HOOK_HMAC_SECRET` |
| Postgres | Events, roster, communities. `pg_dump`, or a quiesced volume snapshot |
| MinIO bucket | Media and Blossom objects |
| `buzz-git-data` volume | `BUZZ_GIT_REPO_PATH=/data/git` |
| Caddy data/config volumes | Issued certificates and ACME account keys |

**The Buzz-specific RPO constraint**, which `run.sh` names and is easy to lose in automation:
**Postgres and the object/git state must come from the same maintenance window.** An event row in
Postgres referencing a media object that the object-store backup predates is a dangling reference.
Snapshot them together or quiesce.

**Properties, per `Research/hardening-linux-servers.md` §3.12** — these decide whether you survive,
and none is in `backup-hint`:

- **Immutability** — Object Lock in compliance mode, so a compromised host cannot delete its own
  backups
- **Credential separation** — the writer role can `Put` but not `Delete`; restore uses a different,
  human-gated credential
- **Keys stored separately from backups**, and separately from the relay-key escrow of §B8, with a
  tested recovery path for the keys themselves
- **Numbers, not adjectives** — measure RPO/RTO in a timed drill and record the measured figure. The
  dev VM is the right place to run the restore drill, and #36's "rebuild a destroyed host from the
  repository" requirement is the same exercise

---

## Part F — Ordering

Two constraints shape this: #5 must not front-run #2, and the plays that can lock you out must be
rehearsed on the VM first.

**Now, without touching #5's roles** — these are cheap, are not hardening roles, and remove real
prod risk:

1. Fix the Docker-repo contradiction in `../ansible/README.md` (delete Design rule 3).
2. Add `BUZZ_COMPOSE_TLS=true` and the three-file Compose invocation to the `compose-bundle` role's
   spec (§B2). This is a bug in the current spec, not new scope — as written, the role deploys a
   plaintext relay to the internet.
3. Set `buzz_admin_host` unset and `buzz_auto_migrate: false` for prod in the variable file (§B1,
   §B6). Two variable values.
4. Extend `scripts/resolve-image-tag.sh` to capture the digest (§B12) — the manifest call already
   happens.
5. Add the Caddy-in-dev change to the SOP (§A.3), which is where the parity model actually gets
   built.

**Then #5, in this order** — access before firewall, because getting locked out mid-sequence is the
failure mode:

6. #29 access + SSH, including break-glass, rehearsed on the VM (§C1, §C2)
7. #30 ingress, then egress (§C3, §C4)
8. #34 Docker daemon + network split (§C10, §B5)
9. #31 sysctl + systemd (§C5, §C6)
10. #32 patching (§C7)
11. #33 logging, audit, time (§C8, §C9)
12. B4, B7, B9, B10, B11 — the application-layer items
13. Part D verification suite, wired into the play as a final gate
14. Part E backups, with a timed restore drill

---

## Part G — Changes this implies for `dev-deployment-SOP.md`

The SOP is the specification, so these belong there rather than only here:

1. **Step 9.5 / Step 13** — the restart command omits `compose.caddy.yml`. As written it is the
   plaintext-relay-on-3000 path (§B2). This is the most important correction.
2. **Steps 9.4, 9.6, 10, 12.1** — reframe around Caddy + `tls internal` + `wss://` (§A.3), which
   removes the `ws://` special case and makes dev exercise prod's code path.
3. **Step 9.6** — add a note that `admin data: [] <- 200` with no credentials is *the expected result
   of a Host-header-only check*, correct on a loopback VM and an unauthenticated disclosure on a
   public one, with a pointer to §B1. Right now it reads as a clean pass.
4. **Step 7.3** — `BUZZ_AUTO_MIGRATE=true` is listed under "leave these alone — they are already
   correct." Correct for dev; wrong for prod (§B6). Say so.
5. **Step 7.3** — add `BUZZ_MAX_CONNECTIONS` and the rate-limit variables to the table, so they are
   explicit rather than inherited defaults (§B9).
6. **Step 7.1 / 7.5** — note that the relay key cannot be rotated and losing it is unrecoverable
   (§B8). "Save both halves somewhere safe" undersells it.
7. **Step 11** — keep the `sleep 1` warning and add that unapproved-key refusal is untested (the SOP
   already admits this in its honesty section; it belongs in the step too) (§B13).
8. **New step, or the hardening runbook** — the break-glass procedure, written before #29 runs
   anywhere (§C1).

The SOP's closing "what has and has not been proven" section is the most valuable thing in it. Keep
that convention here: nothing in this document has been executed. Part B's findings are read from
source and cited; Part C's configurations are from the research doc as corrected by its gap analysis;
the `!override` network syntax in §B5 is the one snippet flagged as needing verification on the
pinned Compose version.

---

## References

**Read from this repository at commit `4dc9604e8`:**

- `crates/buzz-relay/src/api/admin/auth.rs:16-33` — admin authorization is Host + optional Origin
- `crates/buzz-relay/src/api/admin/mod.rs:28-40` — admin routes, including feedback attachments
- `crates/buzz-relay/src/router.rs:60` — admin router mounted only when the host is configured
- `crates/buzz-relay/src/config.rs:910-937` — admin config `None` when `BUZZ_ADMIN_HOST` is empty;
  `BUZZ_ADMIN_WEB_DIR` read only inside that branch
- `crates/buzz-relay/src/config.rs:319-347, 535` — rate-limit and max-connection variables
- `deploy/compose/compose.yml` — published relay port, MinIO root credentials, `buzz-net` as a plain
  bridge, `BUZZ_AUTO_MIGRATE` defaulting false
- `deploy/compose/compose.caddy.yml` — `ports: !reset []`, the control that closes 3000
- `deploy/compose/compose.dev.yml` — Adminer, Prometheus, and datastore ports
- `deploy/compose/run.sh` — `BUZZ_COMPOSE_TLS` default false, `require_env`, `backup-hint`
- `deploy/compose/Caddyfile`, `.env.example` — four-line proxy config; no TLS switch set
- `launchpad/deploy/AGENTS.md`, `ansible/README.md`, `runbooks/dev-deployment-SOP.md`

**External, verified 2026-08-11** (see `Research/hardening-linux-servers-gap-analysis.md` for the
full list):

- [Docker Engine v28: Hardening Container Networking by Default](https://www.docker.com/blog/docker-engine-28-hardening-container-networking-by-default/)
- [moby#35751 — legacy V1 registry support removed](https://github.com/moby/moby/pull/35751)
- [Docker Content Trust retirement — Notary v1 shuts down 8 Dec 2026](https://www.docker.com/blog/docker-content-trust-retirement-and-migration-guidance/)
