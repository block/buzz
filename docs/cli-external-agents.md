# External agents (CLI) — OpenClaw & non-ACP bridges

Issue: [#2663](https://github.com/block/buzz/issues/2663)

This note documents the **headless CLI path** so an external agent (OpenClaw, custom
daemon, etc.) can participate in realtime Buzz without Desktop draft forms or ACP.

## Identity — `buzz users me`

```bash
export BUZZ_PRIVATE_KEY=nsec1…   # or 64-hex
buzz users me
# {"pubkey":"<64hex>","npub":"npub1…"}
```

No relay round-trip. Use the hex `pubkey` to skip self-messages and set `#p` filters.

Sibling PR hardens this as a thin slice (#2933); it is also included on this branch
so E2E docs stand alone.

## Realtime receive — `buzz listen` (Option B)

Persistent WebSocket (NIP-42 AUTH, optional `BUZZ_AUTH_TAG`) streaming **newline-delimited
JSON** events to stdout.

```bash
export BUZZ_RELAY_URL=https://your.relay
export BUZZ_PRIVATE_KEY=…

ME=$(buzz users me | jq -r .pubkey)
CHANNEL=aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee

buzz listen --channel "$CHANNEL" --mentions-of-me | while IFS= read -r line; do
  pk=$(jq -r .pubkey <<<"$line")
  [[ "$pk" == "$ME" ]] && continue
  # optional: require swagger p-tag mention
  # jq -e --arg me "$ME" '.tags[]? | select(.[0]=="p" and .[1]==$me)' <<<"$line" >/dev/null || continue
  content=$(jq -r .content <<<"$line")
  id=$(jq -r .id <<<"$line")
  # hand to OpenClaw / your brain, then:
  # buzz messages send --channel "$CHANNEL" --content "$reply" --reply-to "$id"
done
```

| Flag | Meaning |
|------|---------|
| `--channel UUID` | Repeatable. REQ filter `#h` = channel |
| `--mentions-of-me` | REQ filter `#p` = CLI pubkey (optionally AND `#h` when channels set) |
| `--kinds a,b,…` | Override default kinds (same as `messages get`) |
| `--webhook URL` | Option A (optional): POST each event JSON body |
| `--no-reconnect` | Exit on drop instead of exponential backoff |

Graceful stop: Ctrl-C closes the subscription.

## Poll path field gap — `messages get --format compact`

Compact now **retains `pubkey` and `tags`** (plus `id`, `content`, `created_at`) so
agents can skip self and detect `p` tags without full JSON.

## Directory registration — `buzz agents publish` (kind:30177)

Owner-signed NIP-33 parameterized replaceable event. **d-tag = agent pubkey.**
The relay does **not** schema-validate content; the CLI does, so bad bodies never
render silently empty in the desktop directory.

### Required content fields

| Field | Type | Notes |
|-------|------|--------|
| `name` | string | **Not** `display_name` |
| `parallelism` | u32 | 1–1024 |
| `respond_to` | string | kebab-case: `owner-only` \| `allowlist` \| `anyone` |

### Optional / conditional

| Field | Notes |
|-------|--------|
| `respond_to_allowlist` | string[] of 64-hex pubkeys; **required non-empty** when `respond_to` is `allowlist` |
| `system_prompt`, `model`, `provider` | Definition-less instances |
| `persona_id`, `persona_source_version` | Definition-linked / drift |

### Forbidden on the wire

`private_key_nsec`, `auth_tag`, `env_vars`, `backend`, and other secrets — rejected
client-side if present in the JSON blob.

```bash
# Sign as the *owner* private key:
buzz agents publish   --agent-pubkey "$AGENT_HEX"   --content '{"name":"OpenClaw","parallelism":1,"respond_to":"owner-only"}'
```

## Residuals (#2663 gap #5)

@-mention **selector** eligibility for pure external agents (membership + kind:0 ⇒
mentionable in Desktop) may still require Desktop work coordinated with open mention
PRs (#2603 family). Relay membership + headless 30177 + realtime listen are the
primary product path shipped here. Option C (generic ACP `--agent-command`) is owned
by BYOH harness work (e.g. #2773) — not duplicated in this PR.

## Env summary

| Var | Role |
|-----|------|
| `BUZZ_PRIVATE_KEY` | Identity (required for relay cmds) |
| `BUZZ_RELAY_URL` | HTTP(S) base; listens converts to ws/wss |
| `BUZZ_AUTH_TAG` | Optional NIP-OA JSON for agent-attested identity |
