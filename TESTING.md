# Testing

## Automated Tests

```bash
just test-unit          # unit tests — no infrastructure needed
just test               # unit + integration (starts Docker if needed)
```

`just test` runs unit tests plus integration tests against Postgres and Redis
(started automatically if not already running). Neither task runs the E2E suites in
`sprout-test-client` — those are marked `#[ignore]` and require a running relay:

```bash
# Start a relay first (see below), then:
cargo test -p sprout-test-client -- --ignored
```

---

## Live Local Relay

The fastest way to exercise the relay end-to-end is to build the release
binaries once, run `sprout-relay`, and drive it with the `sprout` CLI. The
CLI signs every request with NIP-98, so you don't need `nak` or hand-rolled
`curl`.

### 1. Setup

```bash
. ./bin/activate-hermit          # activate pinned toolchain
cp .env.example .env             # one-time
just setup                       # start Docker services, run migrations
```

> **Already running Sprout Desktop?** Desktop uses the same Docker container
> names (`sprout-postgres`, `sprout-redis`, `sprout-typesense`) and the same
> default ports (`:5432`, `:6379`, `:8108`). `just setup` will reuse those
> services, so **your test relay writes into Desktop's database**. That's
> fine for read/write smoke tests, but: `just reset` wipes Desktop's data
> along with yours. If you need isolation, stop Desktop first or run the
> dev stack on a different Compose project
> (`COMPOSE_PROJECT_NAME=sprout-dev docker compose …`).

`just reset` wipes all local data and starts over — **including Sprout
Desktop's data** if its services are sharing your dev stack (see callout
above).

> **Heads up — scrub stale env first.** If your shell inherits any of
> `SPROUT_AUTH_TAG`, `SPROUT_RELAY_URL`, or `SPROUT_PRIVATE_KEY` from a
> prior session (or a staging config), `unset` them before continuing.
> A stale `SPROUT_AUTH_TAG` fails the **local dev relay** with
> `auth_error: signature verification failed` on the first CLI write —
> it is *not* tolerated.
> ```bash
> unset SPROUT_AUTH_TAG SPROUT_RELAY_URL SPROUT_PRIVATE_KEY
> ```

### 2. Build the binaries

```bash
cargo build --release -p sprout-relay -p sprout-cli -p sprout-admin
export PATH="$PWD/target/release:$PATH"
```

Rebuild after any code change — the steps below use the release binaries.

### 3. Start the relay

In a separate terminal (it runs in the foreground):

```bash
sprout-relay                     # release binary from step 2, serves ws://localhost:3000
# alternatives:
# cargo run --release -p sprout-relay   # rebuild + run in release
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
> `SPROUT_HEALTH_PORT`) so K8s probes bypass auth middleware. The main app
> port also exposes `/health` for convenience.

The relay starts in dev mode (`SPROUT_REQUIRE_AUTH_TOKEN=false`). The startup
log emits a WARN about this — that's expected for local testing. See the env
vars table at the bottom if you need to lock it down.

> **Already running Sprout Desktop (or another relay) on `:3000` / `:8080` /
> `:9102`?** Sprout binds three ports — main, health, metrics — and any of
> them can collide. Use a separate terminal per role and export the right
> vars in each:
>
> **In the relay terminal** (before launching `sprout-relay`):
> ```bash
> export SPROUT_BIND_ADDR=0.0.0.0:3030
> export SPROUT_HEALTH_PORT=8088
> export SPROUT_METRICS_PORT=9202
> export RELAY_URL=ws://localhost:3030     # advertised in NIP-42 challenges
> sprout-relay
> ```
>
> **In your working / CLI terminal** (for steps 4+ and the ACP harness):
> ```bash
> export SPROUT_RELAY_URL=http://localhost:3030    # CLI target
> # verify the relay on the overridden ports:
> curl -s http://localhost:3030/health             # → ok
> curl -s http://localhost:8088/_readiness         # → {"status":"ready"}
> ```
>
> Every snippet later in this doc shows the defaults. When you see
> `localhost:3000` / `:8080` in a code block, mentally substitute your
> overrides — or the CLI will end up talking to Sprout Desktop's relay.

> **Ignore `just setup`'s "Next steps" banner.** It still prints
> `just relay` (a debug build). Use `sprout-relay` from step 2 here —
> step 2 already built the release binary.

When you're done, stop the relay (Ctrl-C in its terminal). If it's
backgrounded or you lost the terminal: `pkill -f sprout-relay`. Leaving
it running will collide with the next reviewer who follows this doc on
the same machine.

### 4. Smoke test the CLI against the relay

End-to-end: generate an identity, create a channel, post a message, read it
back. This is the minimum sequence an agent needs to verify a local relay.

```bash
# Generate a keypair
GEN=$(sprout-admin generate-key)
export SPROUT_PRIVATE_KEY=$(echo "$GEN" | awk '/Secret key:/ {print $3}')
PUBKEY=$(echo "$GEN"           | awk '/Public key:/ {print $3}')
echo "pubkey: $PUBKEY"

# Create a channel — the UUID is returned in the response
CHANNEL=$(sprout channels create --name "smoke-$$" --type stream --visibility open | jq -r '.channel_id')
echo "channel: $CHANNEL"

# Send a message and read it back
SEND=$(sprout messages send --channel "$CHANNEL" --content "hello from smoke test")
EVENT_ID=$(echo "$SEND" | jq -r '.event_id')
sprout messages get --channel "$CHANNEL" --limit 5 | jq .

# Fetch the reply chain for a specific message (empty array on a leaf — that's fine)
sprout messages thread --channel "$CHANNEL" --event "$EVENT_ID" | jq .
```

A successful run prints `{"event_id":"…","accepted":true,"message":""}` for
the send, and the message body in the `get` output. `thread` returns `[]`
for a leaf message — populated only after a reply comes in (see §5).

### 5. Going deeper

For full coverage of every CLI command (54 subcommands across 12 groups),
follow [`crates/sprout-cli/TESTING.md`](crates/sprout-cli/TESTING.md).

The relay's HTTP bridge accepts three endpoints — useful if you're testing
a client other than `sprout-cli`:

| Endpoint        | Purpose                            |
|-----------------|------------------------------------|
| `POST /events`  | Submit a signed Nostr event        |
| `POST /query`   | NIP-01 filter query (returns events) |
| `POST /count`   | NIP-45 count query                 |

All three accept NIP-98 auth (recommended) or, in dev mode, an `X-Pubkey`
header fallback. There is no REST API for fetching message threads — use
`POST /query` with an `#e` filter, or `sprout messages thread`.

---

## ACP Harness (optional, end-to-end with a real agent)

`sprout-acp` connects an ACP-speaking agent (goose, codex, claude code,
sprout-agent) to the relay. The harness listens for events, drives the
agent over stdio, and the agent replies through MCP tools.

Minimum recipe — assumes the relay from step 3 is running and the channel
`$CHANNEL` from step 4 still exists. The agent identity must be **different**
from the sender identity (`SPROUT_ACP_RESPOND_TO=anyone` still skips events
the agent signed itself).

```bash
cargo build --release -p sprout-acp
export PATH="$PWD/target/release:$PATH"

# 1. Save your sender identity from step 4 — you'll need it to @mention the agent
SENDER_SK="$SPROUT_PRIVATE_KEY"

# 2. Mint a fresh agent identity and capture its pubkey
AGENT_GEN=$(sprout-admin generate-key)
AGENT_SK=$(echo "$AGENT_GEN" | awk '/Secret key:/ {print $3}')
AGENT_PUBKEY=$(echo "$AGENT_GEN" | awk '/Public key:/ {print $3}')

# 3. Add the agent as a member of $CHANNEL — still using the sender identity.
#    Skip this and the agent boots to "discovered 0 channel(s) → agent will
#    sit idle" and silently ignores every mention.
sprout channels add-member --channel "$CHANNEL" --pubkey "$AGENT_PUBKEY" --role member

# 4. Switch to the agent identity and start it.
#    sprout-acp wants ws:// (not http://). If you set SPROUT_RELAY_URL to an
#    http:// URL in step 3, set the ws:// equivalent here — same host/port.
export SPROUT_PRIVATE_KEY="$AGENT_SK"
export SPROUT_RELAY_URL=ws://localhost:3000   # match step 3 (e.g. ws://localhost:3030 if overridden)
export SPROUT_ACP_RESPOND_TO=anyone           # default is owner-only; opens the gate for testing
# NIP-AE core-memory prompt injection is on by default; set SPROUT_ACP_NO_MEMORY=true to opt out.
export GOOSE_MODE=auto                        # must be 'auto' or goose hangs on prompts

sprout-acp                                    # foreground; logs to stdout (run in a separate terminal)

# Optional: turn on per-turn tracing if the default log is too quiet.
# RUST_LOG=sprout_acp=debug sprout-acp
```

> **Using a different ACP agent?** The default recipe assumes `goose` is on
> `$PATH` and configured (`goose --version` should print). For codex / claude
> code / sprout-agent, set `SPROUT_ACP_AGENT_COMMAND` and `SPROUT_ACP_AGENT_ARGS`
> accordingly — see `crates/sprout-acp/README.md`. Without these, sprout-acp
> will fail to spawn the agent subprocess on startup.

If you started the agent before adding it to the channel, just run the
`add-member` afterwards — it picks up the membership notification live and
subscribes without restart (`membership notification: subscribing to new channel …`).

The justfile also ships `just goose key="$AGENT_NSEC"` (foreground) and
`just goose-bg key="$AGENT_NSEC"` (background screen session) which set the
same env. See `crates/sprout-acp/README.md` for parallel agents, heartbeats,
respond-to gates, and forum subscriptions.

Send the agent a task — switch your shell back to the **sender** identity
from step 4 and @mention the agent:

```bash
export SPROUT_PRIVATE_KEY=$SENDER_SK          # the key from step 4
sprout messages send --channel "$CHANNEL" \
  --content "Hey agent, reply PONG only."

# Wait 10–90s, then read the channel — the agent's reply is a kind:9 from
# AGENT_PUBKEY. The current ACP build is quiet on stdout during a turn, so
# `sprout messages get` is how you confirm it ran.
sprout messages get --channel "$CHANNEL" --limit 5 | jq '.[] | {pubkey, content}'
```

Replies are kind:9 in the same channel; `sprout messages thread --channel <id>
--event <event_id>` fetches the reply chain for a specific mention.

---

## Configuration reference

The relay reads all configuration from environment variables. Defaults work
out of the box with `just setup` or `just relay`. Common overrides:

| Variable                          | Default                     | Notes |
|-----------------------------------|-----------------------------|-------|
| `SPROUT_BIND_ADDR`                | `0.0.0.0:3000`              | Main app port |
| `SPROUT_HEALTH_PORT`              | `8080`                      | `/_liveness`, `/_readiness` |
| `SPROUT_METRICS_PORT`             | `9102`                      | Prometheus `/metrics` |
| `RELAY_URL`                       | `ws://localhost:3000`       | Advertised in NIP-11 / NIP-42 challenges. **Note: no `SPROUT_` prefix.** |
| `DATABASE_URL`                    | `postgres://sprout:sprout_dev@localhost:5432/sprout` | |
| `REDIS_URL`                       | `redis://localhost:6379`    | |
| `TYPESENSE_URL`                   | `http://localhost:8108`     | |
| `SPROUT_REQUIRE_AUTH_TOKEN`       | `false`                     | When true, REST requires NIP-98 (no `X-Pubkey` fallback) |
| `SPROUT_REQUIRE_RELAY_MEMBERSHIP` | `false`                     | When true, only pubkeys in `relay_members` can connect |
| `RELAY_OWNER_PUBKEY`              | unset                       | Bootstrapped as `owner` in `relay_members` at first start |
| `SPROUT_ALLOW_NIP_OA_AUTH`        | `false`                     | Enable NIP-OA owner attestation for membership |

CLI-side, only two matter for testing:

| Variable                | Default                  | Notes |
|-------------------------|--------------------------|-------|
| `SPROUT_RELAY_URL`      | `http://localhost:3000`  | CLI relay base; accepts `ws(s)://` and normalises |
| `SPROUT_PRIVATE_KEY`    | — (**required**)         | `nsec1…` or 64-char hex |
| `SPROUT_AUTH_TAG`       | unset                    | Optional NIP-OA owner attestation JSON |

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `relay error 500` or `400: restricted: not a channel member` after a code change | Stale binary | Rebuild and re-export `PATH`; or `cargo run` directly |
| `Address already in use` on relay start (os error 48 on macOS, 98 on Linux) | Another relay (or stale process) holding `:3000` / `:8080` / `:9102` (or your override ports) | The panic line names the failing port — read it first. Then `lsof -iTCP:3000,8080,9102 -sTCP:LISTEN` (or your override equivalents). Kill the offender (`pkill -f sprout-relay`) or use the port-override block in step 3. If you already overrode and *still* collide, a prior reviewer left a relay running on the same alt ports — kill it or pick fresh ports |
| `auth_error: SPROUT_PRIVATE_KEY is required` | Env not exported into the CLI's shell | `export SPROUT_PRIVATE_KEY=...` (or pass `--private-key`) |
| `auth_error: SPROUT_AUTH_TAG verification failed … signature verification failed` | A stale `SPROUT_AUTH_TAG` inherited from a parent shell. The local dev relay rejects it. | `unset SPROUT_AUTH_TAG` (see the scrub block in step 1) |
| `auth-required: verification failed` on a closed relay | NIP-OA attestation needed | Set `SPROUT_AUTH_TAG` to the owner-issued JSON, or relax `SPROUT_REQUIRE_RELAY_MEMBERSHIP` |
| `channels list` empty after `channels create` | The CLI doesn't echo the channel UUID; use the filter shown in step 4 | Or `POST /query` with `{"kinds":[39002]}` |
| ACP agent ignores all events | `SPROUT_ACP_RESPOND_TO=owner-only` (default) with no owner configured | Set `SPROUT_ACP_RESPOND_TO=anyone` for testing |
| ACP logs `discovered 0 channel(s)` / `no channel subscriptions resolved` | Agent identity isn't a member of any channel | `sprout channels add-member --channel "$CHANNEL" --pubkey "$AGENT_PUBKEY" --role member` from another identity |
| `GOOSE_MODE` warning, agent hangs | Not set | `export GOOSE_MODE=auto` |
| Tests pass locally but CI fails | Forgot to run `just ci` | `just ci` runs the gate (fmt, clippy, unit tests, desktop/web builds) |
