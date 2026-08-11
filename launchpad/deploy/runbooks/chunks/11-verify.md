# Chunk 11 — verify

**What it does.** Runs SOP Step 17's checklist as executable assertions rather than by eye, plus the
runnable subset of `hardening-spec.md` Part D. It fails rather than warns, and the network checks run
**from the host** because on-target checks lie about container reachability.

**SOP steps covered:** 17, minus the agent items (19–26), which are out of scope. Part D's rules are
in `hardening-spec.md`.

## Preconditions

- Chunk 07 at minimum. Assertions whose chunk has not run **skip** rather than fail, so it is useful
  part-way through: before chunk 10 the hardening block skips and the summary still reports 0 failed.
- `python3` on the host (used to parse `docker compose ps --format json`, which emits one object per
  line rather than an array).

## Run

```bash
./deploy verify
```

Against another target:

```bash
TARGET=vps SSH_HOST=<host> SSH_PORT=22 SSH_USER=<user> \
  DOMAIN=<domain> AUTHORITY=<domain> ADMIN_AUTHORITY= HTTPS_PORT=443 \
  ./scripts/verify.sh
```

Note `AUTHORITY=<domain>` with no port for production — the relay strips a trailing `:443` — and an
empty `ADMIN_AUTHORITY`, which is the production posture (`hardening-spec.md` §B1).

## Verify

Exit status is the result: `0` means every assertion passed. The summary line reports
passed/failed/skipped, and failures are listed again at the end.

Measured on 2026-08-12 against the TLS path before hardening: **19 passed, 0 failed, 2 skipped**
(both skips were the hardening block, correctly, because chunk 10 had not run).

## Rollback

Read-only. It changes nothing and has nothing to roll back.

## Traps

- **A clean run does NOT mean production-hardened.** This is the dev-VM subset. Part D assertions
  deliberately not implemented here: off-host `nmap`, default-deny **egress**, datastore network
  isolation (§B5), the MinIO service account (§B4), image digest pinning (§B12), TLS grade via
  `testssl.sh`, systemd exposure score (§C6), and the timed restore drill (§E). The script prints this
  list on both success and failure so it cannot be read as more than it is.
- **Two assertions differ by target on purpose**, and that difference is the point rather than a bug:
  the admin API answers `200` unauthenticated in dev and must be **absent** in production, and
  `BUZZ_AUTO_MIGRATE` is `true` in dev and `false` in production (§B1, §B6).
- **Empty `Health` in `docker compose ps` means "declares no healthcheck", not "unhealthy".** Caddy
  is that case. An earlier version of this suite demanded `healthy` for every service and failed on a
  perfectly good stack.
- **`net.ipv4.ip_forward` is asserted to be `1`, not `0`.** Docker requires forwarding and re-enables
  it, so asserting `0` — as most hardening guides suggest — fails forever (§C5).
- **The secret scan is scoped to `launchpad/deploy` and matches real key shapes.** SOP Step 17 item 27
  specifies `git grep -nE 'sk-or-v1-|OPENROUTER_API_KEY=sk'` across the whole repo, and **that check
  fails on a clean checkout**: it matches the SOP's own prose, `buzz-agent/README.md`'s example, and
  `desktop/src-tauri/src/commands/agent_models_tests.rs`, which uses `sk-or-v1-secret-key-12345` as a
  fixture in a test *for secret redaction*. A check that cries wolf every run gets ignored.
- **Do not test repository hygiene as "`git status` is clean."** Activating Hermit legitimately
  modifies `bin/pnpm`, so the tree is dirty for reasons that have nothing to do with secrets.
- **`sshd -T`, never the file.** The suite reads the merged effective config, because sshd keeps the
  first value it reads and a correct-looking file can be losing to `50-cloud-init.conf` or
  `60-cloudimg-settings.conf` (§C2). Both of those are confirmed present on this image.
