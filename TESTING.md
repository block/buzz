# Testing

Live local relay, ACP harness, and [PR screenshot](#pr-screenshots) runbooks. Agent hard rules stay in [AGENTS.md](AGENTS.md).

## Automated Tests

```bash
just test-unit          # unit tests — no infrastructure needed
just test               # unit + integration (starts Docker if needed)
```

`just test` runs unit tests plus integration tests against Postgres and Redis
(started automatically if not already running). Neither task runs the E2E suites in
`buzz-test-client` — those are marked `#[ignore]` and require a running relay:

```bash
# Start a relay first (see below), then:
cargo test -p buzz-test-client -- --ignored
```

---

## Live Local Relay

The fastest way to exercise the relay end-to-end is to build the release
binaries once, run `buzz-relay`, and drive it with the `buzz` CLI. The
CLI signs every request with NIP-98, so you don't need `nak` or hand-rolled
`curl`.

### 1. Setup

```bash
. ./bin/activate-hermit          # activate pinned toolchain
cp .env.example .env             # one-time
just setup                       # start Docker services, run migrations
```

> **Already running Pkzz Desktop?** Desktop uses the same Docker container
> names (`buzz-postgres`, `buzz-redis`) and the same
> default ports (`:5432`, `:6379`). `just setup` will reuse those
> services, so **your test relay writes into Desktop's database**. That's
> fine for read/write smoke tests, but: `just reset` wipes Desktop's data
> along with yours. If you need isolation, stop Desktop first or run the
> dev stack on a different Compose project
> (`COMPOSE_PROJECT_NAME=buzz-dev docker compose …`).

`just reset` wipes all local data and starts over — **including Pkzz
Desktop's data** if its services are sharing your dev stack (see callout
above).

> **Heads up — scrub stale env first.** If your shell inherits any of
> `BUZZ_AUTH_TAG`, `BUZZ_RELAY_URL`, or `BUZZ_PRIVATE_KEY` from a
> prior session (or a staging config), `unset` them before continuing.
> A stale `BUZZ_AUTH_TAG` fails the **local dev relay** with
> `auth_error: signature verification failed` on the first CLI write —
> it is *not* tolerated.
> ```bash
> unset BUZZ_AUTH_TAG BUZZ_RELAY_URL BUZZ_PRIVATE_KEY
> ```

### 2. Build the binaries

```bash
cargo build --release -p buzz-relay -p buzz-cli -p buzz-admin
export PATH="$PWD/target/release:$PATH"
```

Rebuild after any code change — the steps below use the release binaries.

### 3. Start the relay

In a separate terminal (it runs in the foreground):

```bash
buzz-relay                     # release binary from step 2, serves ws://localhost:3000
# alternatives:
# cargo run --release -p buzz-relay     # rebuild + run in release
# just relay                            # DEBUG build — fast to launch on a hot cache,
#                                       # but mismatched if step 2 left you on release.
#                                       # Use `just relay-release` if you want the recipe.
```

Verify it's up (back in your working terminal):

```bash
curl -s http://localhost:3000/health           # → ok
curl -s http://localhost:8080/_readiness        # → {"status":"ready"}
```

> Health/readiness/liveness live on a **separate port** (default `8080`,
> `BUZZ_HEALTH_PORT`) so K8s probes bypass auth middleware. The main app
> port also exposes `/health` for convenience.

The relay starts in dev mode (`BUZZ_REQUIRE_AUTH_TOKEN=false`). The startup
log emits a WARN about this — that's expected for local testing. See the env
vars table at the bottom if you need to lock it down.

> **Already running Pkzz Desktop (or another relay) on `:3000` / `:8080` /
> `:9102`?** Pkzz binds three ports — main, health, metrics — and any of
> them can collide. Use a separate terminal per role and export the right
> vars in each:
>
> **In the relay terminal** (before launching `buzz-relay`):
> ```bash
> export BUZZ_BIND_ADDR=0.0.0.0:3030
> export BUZZ_HEALTH_PORT=8088
> export BUZZ_METRICS_PORT=9202
> export RELAY_URL=ws://localhost:3030     # advertised in NIP-42 challenges
> buzz-relay
> ```
>
> **In your working / CLI terminal** (for steps 4+ and the ACP harness):
> ```bash
> export BUZZ_RELAY_URL=http://localhost:3030    # CLI target
> # verify the relay on the overridden ports:
> curl -s http://localhost:3030/health             # → ok
> curl -s http://localhost:8088/_readiness         # → {"status":"ready"}
> ```
>
> Every snippet later in this doc shows the defaults. When you see
> `localhost:3000` / `:8080` in a code block, mentally substitute your
> overrides — or the CLI will end up talking to Pkzz Desktop's relay.

> **Ignore `just setup`'s "Next steps" banner.** It still prints
> `just relay` (a debug build). Use `buzz-relay` from step 2 here —
> step 2 already built the release binary.

When you're done, stop the relay (Ctrl-C in its terminal). If it's
backgrounded or you lost the terminal: `pkill -f buzz-relay`. Leaving
it running will collide with the next reviewer who follows this doc on
the same machine.

### 4. Smoke test the CLI against the relay

End-to-end: generate an identity, create a channel, post a message, read it
back. This is the minimum sequence an agent needs to verify a local relay.

```bash
# Generate a keypair
GEN=$(buzz-admin generate-key)
export BUZZ_PRIVATE_KEY=$(echo "$GEN" | awk '/Secret key:/ {print $3}')
PUBKEY=$(echo "$GEN"           | awk '/Public key:/ {print $3}')
echo "pubkey: $PUBKEY"

# Create a channel — the UUID is returned in the response
CHANNEL=$(buzz channels create --name "smoke-$$" --type stream --visibility open | jq -r '.channel_id')
echo "channel: $CHANNEL"

# Send a message and read it back
SEND=$(buzz messages send --channel "$CHANNEL" --content "hello from smoke test")
EVENT_ID=$(echo "$SEND" | jq -r '.event_id')
buzz messages get --channel "$CHANNEL" --limit 5 | jq .

# Fetch the reply chain for a specific message (empty array on a leaf — that's fine)
buzz messages thread --channel "$CHANNEL" --event "$EVENT_ID" | jq .
```

A successful run prints `{"event_id":"…","accepted":true,"message":""}` for
the send, and the message body in the `get` output. `thread` returns `[]`
for a leaf message — populated only after a reply comes in (see §5).

### 5. Going deeper

For full coverage of every CLI command (54 subcommands across 12 groups),
follow [`crates/buzz-cli/TESTING.md`](crates/buzz-cli/TESTING.md).

The relay's HTTP bridge accepts three endpoints — useful if you're testing
a client other than `buzz-cli`:

| Endpoint        | Purpose                            |
|-----------------|------------------------------------|
| `POST /events`  | Submit a signed Nostr event        |
| `POST /query`   | NIP-01 filter query (returns events) |
| `POST /count`   | NIP-45 count query                 |

All three accept NIP-98 auth (recommended) or, in dev mode, an `X-Pubkey`
header fallback. There is no REST API for fetching message threads — use
`POST /query` with an `#e` filter, or `buzz messages thread`.

---

## ACP Harness (optional, end-to-end with a real agent)

`buzz-acp` connects an ACP-speaking agent (goose, codex, claude code,
buzz-agent) to the relay. The harness listens for events, drives the
agent over stdio, and the agent replies through MCP tools.

Minimum recipe — assumes the relay from step 3 is running and the channel
`$CHANNEL` from step 4 still exists. The agent identity must be **different**
from the sender identity (`BUZZ_ACP_RESPOND_TO=anyone` still skips events
the agent signed itself).

```bash
cargo build --release -p buzz-acp
export PATH="$PWD/target/release:$PATH"

# 1. Save your sender identity from step 4 — you'll need it to @mention the agent
SENDER_SK="$BUZZ_PRIVATE_KEY"

# 2. Mint a fresh agent identity and capture its pubkey
AGENT_GEN=$(buzz-admin generate-key)
AGENT_SK=$(echo "$AGENT_GEN" | awk '/Secret key:/ {print $3}')
AGENT_PUBKEY=$(echo "$AGENT_GEN" | awk '/Public key:/ {print $3}')

# 3. Add the agent as a member of $CHANNEL — still using the sender identity.
#    Skip this and the agent boots to "discovered 0 channel(s) → agent will
#    sit idle" and silently ignores every mention.
buzz channels add-member --channel "$CHANNEL" --pubkey "$AGENT_PUBKEY" --role member

# 4. Switch to the agent identity and start it.
#    buzz-acp wants ws:// (not http://). If you set BUZZ_RELAY_URL to an
#    http:// URL in step 3, set the ws:// equivalent here — same host/port.
export BUZZ_PRIVATE_KEY="$AGENT_SK"
export BUZZ_RELAY_URL=ws://localhost:3000   # match step 3 (e.g. ws://localhost:3030 if overridden)
export BUZZ_ACP_RESPOND_TO=anyone           # default is owner-only; opens the gate for testing
# NIP-AE core-memory prompt injection is on by default; set BUZZ_ACP_NO_MEMORY=true to opt out.
export GOOSE_MODE=auto                        # must be 'auto' or goose hangs on prompts

buzz-acp                                    # foreground; logs to stdout (run in a separate terminal)

# Optional: turn on per-turn tracing if the default log is too quiet.
# RUST_LOG=buzz_acp=debug buzz-acp
```

> **Using a different ACP agent?** The default recipe assumes `goose` is on
> `$PATH` and configured (`goose --version` should print). For codex / claude
> code / buzz-agent, set `BUZZ_ACP_AGENT_COMMAND` and `BUZZ_ACP_AGENT_ARGS`
> accordingly — see `crates/buzz-acp/README.md`. Without these, buzz-acp
> will fail to spawn the agent subprocess on startup.

If you started the agent before adding it to the channel, just run the
`add-member` afterwards — it picks up the membership notification live and
subscribes without restart (`membership notification: subscribing to new channel …`).

The justfile also ships `just goose key="$AGENT_NSEC"` (foreground) and
`just goose-bg key="$AGENT_NSEC"` (background screen session) which set the
same env. See `crates/buzz-acp/README.md` for parallel agents, heartbeats,
respond-to gates, and forum subscriptions.

To exercise deferred ACP startup, add `BUZZ_ACP_LAZY_POOL=true` before launching
`buzz-acp`. The harness should connect, authenticate, subscribe, and publish
online presence without starting the configured ACP child. The first accepted,
flushable mention should start exactly one child and then dispatch the queued
message. Automated coverage in `pool_lifecycle_state` pins single-wake,
retry/backoff, and stale-result behavior; it does not replace this real
relay/process smoke test.

Send the agent a task — switch your shell back to the **sender** identity
from step 4 and @mention the agent:

```bash
export BUZZ_PRIVATE_KEY=$SENDER_SK          # the key from step 4
buzz messages send --channel "$CHANNEL" \
  --content "Hey agent, reply PONG only."

# Wait 10–90s, then read the channel — the agent's reply is a kind:9 from
# AGENT_PUBKEY. The current ACP build is quiet on stdout during a turn, so
# `buzz messages get` is how you confirm it ran.
buzz messages get --channel "$CHANNEL" --limit 5 | jq '.[] | {pubkey, content}'
```

Replies are kind:9 in the same channel; `buzz messages thread --channel <id>
--event <event_id>` fetches the reply chain for a specific mention.

---

## Configuration reference

The relay reads all configuration from environment variables. Defaults work
out of the box with `just setup` or `just relay`. Common overrides:

| Variable                          | Default                     | Notes |
|-----------------------------------|-----------------------------|-------|
| `BUZZ_BIND_ADDR`                | `0.0.0.0:3000`              | Main app port |
| `BUZZ_HEALTH_PORT`              | `8080`                      | `/_liveness`, `/_readiness` |
| `BUZZ_METRICS_PORT`             | `9102`                      | Prometheus `/metrics` |
| `RELAY_URL`                       | `ws://localhost:3000`       | Advertised in NIP-11 / NIP-42 challenges. **Note: no `BUZZ_` prefix.** |
| `DATABASE_URL`                    | `postgres://buzz:buzz_dev@localhost:5432/buzz` | |
| `REDIS_URL`                       | `redis://localhost:6379`    | |
| `BUZZ_REQUIRE_AUTH_TOKEN`       | `false`                     | When true, REST requires NIP-98 (no `X-Pubkey` fallback) |
| `BUZZ_REQUIRE_RELAY_MEMBERSHIP` | `false`                     | When true, only pubkeys in `relay_members` can connect |
| `BUZZ_REQUIRE_MEDIA_GET_AUTH`   | `false`                     | When true, `GET`/`HEAD /media/*` require Blossom kind 24242 `t=get` auth plus relay membership. |
| `BUZZ_DRAIN_JITTER_MS`          | `0` (off)                   | Per-connection upper bound, in ms, for the random delay before each live WebSocket gets its `1012 Service Restart` close on graceful shutdown. `0` closes every socket at once (the previous behavior). A positive value spreads closes uniformly over `[1, value]` ms to avoid a reconnect thundering herd on rolling deploys. Values above `20000` are capped to `20000` (`MAX_DRAIN_JITTER_MS`) to leave close-frame delivery headroom under the relay's 30s hard-drain timeout. Empty or whitespace-only is treated as unset (off); a non-integer fails startup loudly. |
| `BUZZ_AUDIT_ENABLED`            | `true`                      | Tamper-evident event/media audit log. Set `false`/`0`/`off` to skip its DB pool and writes. Does not disable the separate moderation audit trail. |
| `BUZZ_AUTO_MIGRATE`             | `false`                     | Opt in with `true`/`1`/`yes`/`on` to run embedded SQLx migrations on relay startup |
| `RELAY_OWNER_PUBKEY`              | unset                       | Bootstrapped as `owner` in `relay_members` at first start |
| `BUZZ_ALLOW_NIP_OA_AUTH`        | `false`                     | Enable NIP-OA owner attestation for membership |
| `BUZZ_WEB_DIR`                  | unset (source), `/srv/buzz/web` (container) | Directory containing the invite landing bundle; the production container enables it so `/invite/{code}` always works |
| `BUZZ_SERVE_GIT_WEB_GUI`        | `false`                     | Set to `true` or `1` to expose the bundled Git repository browser at `/` and `/repos/...`; invite routes do not depend on this flag |

CLI-side, only two matter for testing:

| Variable                | Default                  | Notes |
|-------------------------|--------------------------|-------|
| `BUZZ_RELAY_URL`      | `http://localhost:3000`  | CLI relay base; accepts `ws(s)://` and normalises |
| `BUZZ_PRIVATE_KEY`    | — (**required**)         | `nsec1…` or 64-char hex |
| `BUZZ_AUTH_TAG`       | unset                    | Optional NIP-OA owner attestation JSON |

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `relay error 500` or `400: restricted: not a channel member` after a code change | Stale binary | Rebuild and re-export `PATH`; or `cargo run` directly |
| `Address already in use` on relay start (os error 48 on macOS, 98 on Linux) | Another relay (or stale process) holding `:3000` / `:8080` / `:9102` (or your override ports) | The panic line names the failing port — read it first. Then `lsof -iTCP:3000,8080,9102 -sTCP:LISTEN` (or your override equivalents). Kill the offender (`pkill -f buzz-relay`) or use the port-override block in step 3. If you already overrode and *still* collide, a prior reviewer left a relay running on the same alt ports — kill it or pick fresh ports |
| `auth_error: BUZZ_PRIVATE_KEY is required` | Env not exported into the CLI's shell | `export BUZZ_PRIVATE_KEY=...` (or pass `--private-key`) |
| `auth_error: BUZZ_AUTH_TAG verification failed … signature verification failed` | A stale `BUZZ_AUTH_TAG` inherited from a parent shell. The local dev relay rejects it. | `unset BUZZ_AUTH_TAG` (see the scrub block in step 1) |
| `auth-required: verification failed` on a closed relay | NIP-OA attestation needed | Set `BUZZ_AUTH_TAG` to the owner-issued JSON, or relax `BUZZ_REQUIRE_RELAY_MEMBERSHIP` |
| `channels list` empty after `channels create` | The CLI doesn't echo the channel UUID; use the filter shown in step 4 | Or `POST /query` with `{"kinds":[39002]}` |
| ACP agent ignores all events | `BUZZ_ACP_RESPOND_TO=owner-only` (default) with no owner configured | Set `BUZZ_ACP_RESPOND_TO=anyone` for testing |
| ACP logs `discovered 0 channel(s)` / `no channel subscriptions resolved` | Agent identity isn't a member of any channel | `buzz channels add-member --channel "$CHANNEL" --pubkey "$AGENT_PUBKEY" --role member` from another identity |
| `GOOSE_MODE` warning, agent hangs | Not set | `export GOOSE_MODE=auto` |
| Tests pass locally but CI fails | Forgot to run `just ci` | `just ci` runs the gate (fmt, clippy, unit tests, desktop/web builds) |

---

## PR Screenshots

Capture and post UI screenshots for pull requests. Do not host these images on the relay or a third-party host — that rule is also in [AGENTS.md § Testing](AGENTS.md#testing).

> **Do NOT use `buzz upload`, the relay media endpoint, or any third-party
> image host for PR screenshots.** Relay media URLs fail through GitHub's camo
> proxy. Always use `scripts/post-screenshots.sh` for PNGs before linking them
> from a PR body/comment. If you hand-edit PR markdown, run
> `scripts/check-pr-image-urls.sh <markdown-file>` first to catch relay URLs.

For mobile simulator screenshots, save the PNGs in a local directory and run
`./scripts/post-screenshots.sh <PR-number> <png-dir>` or use the third argument
with a markdown template containing `{{filename}}` placeholders.

The desktop app requires the E2E mock bridge to render — it cannot run in a plain
browser. Use `just desktop-screenshot` to capture screenshots (builds frontend,
starts preview server, runs Playwright automatically):

```bash
just desktop-screenshot --name home
just desktop-screenshot --name channel --route /channels/general
just desktop-screenshot --name search --click open-search
just desktop-screenshot --name settings --click open-settings
```

Options: `--name` (filename), `--route` (client route), `--active-channel`
(channel to view), `--click` (left-click data-testid or CSS selector),
`--right-click` (right-click for context menus), `--hover` (hover before
capture), `--clip` (crop region as `x,y,w,h` — e.g. `0,0,256,720` for sidebar
only), `--wait` (ms, default 2000), `--viewport` (WxH, default 1280x720),
`--outdir` (default `test-results/screenshots`), `--messages` (JSON file path).
Output is a PNG path on stdout.

Use `--messages` to inject content into a channel before capture. The JSON file
is an array of objects — `channelName` and `content` are required, all other
fields are optional and passed through to `__BUZZ_E2E_EMIT_MOCK_MESSAGE__`:

```json
[
  {
    "channelName": "random",
    "content": "Hey @tyler check this out",
    "pubkey": "953d...",
    "kind": 40002,
    "mentionPubkeys": ["deadbeef..."],
    "extraTags": [["broadcast", "1"], ["e", "some-root-id"]],
    "parentEventId": "abc123"
  }
]
```

Without `--active-channel`, all messages must target the same channel and the
helper navigates to that channel (useful for showing message content). With
`--active-channel`, messages can target multiple channels while the "camera"
stays on the specified channel (useful for unread indicators, badges, etc.).

```bash
# Messages in the channel you're viewing (code blocks, formatting, etc.)
just desktop-screenshot --name code-blocks --messages /tmp/msgs.json

# Messages in OTHER channels to trigger unread state
just desktop-screenshot --name unread-dot \
  --active-channel general --messages /tmp/badge-msgs.json

# Cropped to sidebar only (256px wide)
just desktop-screenshot --name sidebar-unread \
  --active-channel general --messages /tmp/badge-msgs.json \
  --clip 0,0,256,720

# Context menu on an unread channel (wider crop to include popup)
just desktop-screenshot --name ctx-mark-read \
  --active-channel general --messages /tmp/badge-msgs.json \
  --right-click channel-random --clip 0,200,320,300

# Hover state (e.g. copy button reveal)
just desktop-screenshot --name copy-hover \
  --messages /tmp/code-msgs.json --hover "[data-testid='copy-code']"
```

Available mock channels: `general`, `random`, `design`, `sales`, `engineering`,
`agents`, `watercooler`, `announcements`, `alice-tyler`, `bob-tyler`.

`scripts/post-screenshots.sh` hosts PNGs on a per-developer branch
(`agent-screenshots/<github-username>`) and posts a PR comment with
commit-SHA-based image URLs (immutable — safe from later overwrites):

```bash
./scripts/post-screenshots.sh 803 test-results/screenshots
./scripts/post-screenshots.sh 803 test-results/screenshots body.md  # custom body prepended
```

The body file supports `{{filename}}` placeholders (without `.png`) to inline
images at specific positions. Images not referenced by any placeholder are
appended at the end. Without placeholders, all images are appended (backward
compatible).

```markdown
### Unread dot
A message arrives in `#random`.

{{01-unread-dot}}

### Context menu
Right-click shows "Mark as read".

{{02-context-menu}}
```

Re-runs overwrite the image blobs on the `agent-screenshots/<username>`
branch, but the script **appends a new PR comment** — it does not edit or
delete the previous one. After reposting, delete the superseded comment so
only the current set remains, otherwise reviewers still see the stale images:

```bash
# List screenshot comments to find the stale one's id
gh pr view <pr> --repo kingkillery/pkzz --json comments \
  --jq '.comments[] | select(.body | test("pr-<pr>--")) | {id, url}'
gh api -X DELETE repos/kingkillery/pkzz/issues/comments/<stale-comment-id>
```

Branch cleanup when fully done: `git push origin --delete agent-screenshots/<username>`.

### Writing E2E Screenshot Specs

When screenshots need seeded state, live messages, or UI interaction before
capture, write a Playwright spec instead of using `just desktop-screenshot`.
Add specs to `desktop/tests/e2e/` and register them in `desktop/playwright.config.ts`
(`smoke` project `testMatch`). Every test calls `installMockBridge(page)` for
mock Tauri IPC. Mock pubkey, channel names, and UUIDs live in `desktop/src/testing/e2eBridge.ts`.

**Always build with `pnpm build:e2e`, never `pnpm run build`.** The mock Tauri
bridge is compiled in only for `--mode e2e` (see `installE2eBridgeIfConfigured`
in `desktop/src/main.tsx`). A plain `pnpm run build` strips it, so
`window.__TAURI_INTERNALS__` is never defined and **every** mock-mode spec fails
with `Cannot read properties of undefined (reading 'invoke')` — the app renders
"Community connection failed" instead of the UI under test. That looks exactly
like a product bug rather than a build mistake, so it burns real time.
`pnpm test:e2e:smoke` and `pnpm test:e2e:integration` run the right build for
you; prefer them over a manual build plus `playwright test`.

**Stale server:** `desktop/playwright.config.ts` sets
`reuseExistingServer: !process.env.CI`, so local runs can reuse a previous
build's server and serve stale code; CI does not reuse a server. Kill port 4173
and re-run `pnpm build:e2e` before re-running tests after code changes.

**`addInitScript` before bridge:** `page.addInitScript` (localStorage seeding)
must run BEFORE `installMockBridge(page)` — React reads state on mount, the
bridge triggers mount.

**Live messages:** Call `waitForMockLiveSubscription(page, channelName)` before
`__BUZZ_E2E_EMIT_MOCK_MESSAGE__` — messages are silently dropped without a
subscription. Navigate to the channel first (triggers subscription), then away
(so unread indicators appear), then inject.

**Animation timing:** Radix components animate in via CSS. `toBeVisible()`
resolves mid-animation — wait for completion before screenshotting. Use the
shared helper (mandatory before any `page.screenshot()` or
`locator.screenshot()` in specs):

```ts
import { waitForAnimations } from "../helpers/animations";

// ... after the element is visible but before capturing:
await waitForAnimations(page);
await page.screenshot({ path: "...", clip: { ... } });
```

The `just desktop-screenshot` path (`desktop/tests/helpers/screenshot.mjs`) calls
`waitForAnimations` automatically — no manual step needed there.

For per-element waits (rare — prefer the page-level helper above):

```ts
await menuItem.evaluate((el) =>
  Promise.all(
    el.closest("[data-state]")?.getAnimations().map((a) => a.finished) ?? [],
  ),
);
```

**Cropping:** Use `clip` — full-window (1280x720) screenshots are unreadable
for sidebar features. Sidebar = 256px; context menus ~450px.

**Distinct states — verify before posting:** when one view renders many
elements at once (e.g. all team cards in a single grid), an unscoped
full-page `page.screenshot()` captures the *same* pixels for every shot, so
multiple PNGs come out byte-identical. Scope each shot to its subject with
`locator.screenshot()` (full-page `clip` only when an overlay like an open
dropdown must be included). Then gate on hash distinctness before posting:

```bash
shasum -a 256 test-results/<dir>/*.png   # every hash must be unique
```

Identical hashes mean two shots captured the same state — fix the spec, do
not post. This catches the most common screenshot regression.

**`general` has pre-seeded messages** making `hasUnread` always true. Use
`engineering` for "muted + no unread" visual states.

**PR comments:** Use a body template (3rd arg to `post-screenshots.sh`) with
`{{filename}}` placeholders. Each screenshot gets a `###` heading + one-line
description. See [PR #803](https://github.com/kingkillery/pkzz/pull/803).
