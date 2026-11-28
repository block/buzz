# Last30Days Agent (example)

A focused, **provider-agnostic** multi-worker research example for Buzz.

It shows how to implement `/last30days <topic>` **without** adding command
semantics to Buzz core. Slash routing already passes commands through to ACP
connectors ([#919](https://github.com/block/buzz/pull/919)); discovery and
autocomplete are tracked separately
([#2528](https://github.com/block/buzz/issues/2528),
[#3537](https://github.com/block/buzz/pull/3537)).

This package is the flagship **examples-only** reference proposed in
[#4158](https://github.com/block/buzz/issues/4158).

## What it does

1. Accepts a research topic (CLI, persona pack, or Buzz slash pass-through).
2. Optionally gathers an evidence brief via an adopter-supplied JSON argv command (`shell=False`).
3. Fans out **10 independent workers** (default) over an OpenAI-compatible Chat
   Completions API, each locked to a fixed perspective.
4. Runs one synthesis call and publishes a structured brief only if the
   min-success threshold is met (default: all 10 workers must return usable
   `message.content` — reasoning-only replies do **not** count).
5. Optionally enforces shareability gates: concurrency lock, event-id
   idempotency, per-requester cooldown/quota, and a daily spend ceiling
   (worst-case reservation, not a post-hoc spent check).

Documented default model slug: **`deepseek/deepseek-v4-pro`** via an
OpenRouter-compatible base URL. Adopters supply their own API key and may point
base URL / model / evidence command anywhere.

## Non-goals

- No relay, Desktop UI, or core slash routing changes.
- No bundled credentials, env-file discovery, or personal host paths.
- No direct Nostr/WebSocket relay watcher (that would not exercise #919).
- No guarantee that every OpenAI-compatible provider supports web search;
  evidence acquisition is an explicit, documented optional backend.

## Layout (meadow-core OPS persona-pack shape)

Matches [`meadow-core`](../meadow-core/) pack conventions
(`.plugin/plugin.json`, `agents/*.persona.md`, `skills/*/SKILL.md`,
`instructions.md`) plus a small scripts/ CLI for the multi-worker swarm.

```
last30days-agent/
├── .plugin/
│   └── plugin.json              # OPS-compatible pack manifest
├── agents/
│   └── last30days.persona.md    # Persona advertising /last30days
├── skills/
│   └── last30days/
│       └── SKILL.md             # Orchestrator + publication skill
├── scripts/
│   ├── last30days.py            # Multi-worker orchestrator CLI
│   └── test_last30days.py       # Offline mocked regressions (no network)
├── instructions.md              # Pack-wide instructions
├── .env.example                 # Placeholder names only (never auto-loaded)
└── README.md                    # this file
```

Precedent files matched:

- `examples/meadow-core/.plugin/plugin.json`
- `examples/meadow-core/agents/*.persona.md`
- `examples/meadow-core/skills/*/SKILL.md`
- `examples/meadow-core/instructions.md`
- `examples/countdown-bot/` (#516) for small runnable reference style

## Quickstart (CLI)

```bash
# From a Buzz checkout, pack root, or absolute pack install path
export OPENAI_API_KEY="your-key"   # or LAST30DAYS_API_KEY
# Optional: OpenRouter is the documented default base URL
# export OPENAI_BASE_URL="https://openrouter.ai/api/v1"
# export LAST30DAYS_MODEL="deepseek/deepseek-v4-pro"

# Preferred: opaque topic via stdin (never shell-quote untrusted topic text)
printf '%s' 'Buzz agent collaboration' | \
  python3 examples/last30days-agent/scripts/last30days.py \
    --topic-stdin --skip-evidence

# Or: topic file
# python3 examples/last30days-agent/scripts/last30days.py \
#   --topic-file /tmp/topic.txt --skip-evidence

# Offline tests (no key, no network)
python3 examples/last30days-agent/scripts/test_last30days.py
```

Artifacts land under `./.last30days-runs/` (mode `0700`) with per-file mode
`0600`: `worker-*.md`, `brief.md`, `receipt.json` (metadata only), and
`run-context.json` (private topic/paths/gate identity). Receipts never include
Authorization headers, keys, topic text, or brief body.

## Configuration

| Variable | Default | Notes |
|----------|---------|-------|
| `LAST30DAYS_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` | *(required)* | First non-empty wins. Env only — no file discovery. |
| `LAST30DAYS_BASE_URL` / `OPENAI_BASE_URL` | `https://openrouter.ai/api/v1` | Any OpenAI-compatible base. **Changing this sends the API key to that host** — require explicit operator trust. |
| `LAST30DAYS_MODEL` / `OPENAI_MODEL` | `deepseek/deepseek-v4-pro` | Any chat model id. |
| `LAST30DAYS_WORKERS` | `10` | Independent perspectives. |
| `LAST30DAYS_MIN_SUCCESS` | `10` | Owner/debug only. Under `--enforce-gates`, min-success is forced equal to the worker count. |
| `LAST30DAYS_REASONING` | `high` | Passed when the provider supports reasoning effort. |
| `LAST30DAYS_WORKER_MAX_TOKENS` | `4096` | Headroom vs reasoning-only truncation. |
| `LAST30DAYS_SYNTH_MAX_TOKENS` | `6144` | Synthesis budget. |
| `LAST30DAYS_EVIDENCE_CMD` | unset | JSON argv array only (e.g. `["tool","--topic","{topic}"]`); `shell=False`. |
| `LAST30DAYS_STATE_DIR` | `./.last30days-runs` | Run artifacts (never a hardcoded home path). |
| `LAST30DAYS_GATES_DIR` | `$STATE_DIR/gates` | Cooldown / quota / spend / lock files. |
| `LAST30DAYS_COOLDOWN_S` | `300` | Per-requester cooldown (with `--enforce-gates`). |
| `LAST30DAYS_DAILY_QUOTA` | `5` | Per-requester runs / UTC day. |
| `LAST30DAYS_DAILY_SPEND_USD` | `5.0` | Global spend ceiling. |
| `LAST30DAYS_RESERVE_USD` | `0.50` | Worst-case reservation per run. |
| `LAST30DAYS_MAX_CONCURRENT` | `1` | Global concurrency. |
| `LAST30DAYS_MAX_TOPIC_CHARS` | `500` | Cap under `--enforce-gates`. |

## Shareability gates

For channel-facing / shared use, pass identities and enable gates:

```bash
printf '%s' 'topic words' | python3 examples/last30days-agent/scripts/last30days.py \
  --topic-stdin \
  --enforce-gates \
  --event-id <64-hex-buzz-event-id> \
  --requester <64-hex-pubkey> \
  --channel <channel-uuid>
```

Under `--enforce-gates`:

- Identity shapes are validated (64-hex event/requester, UUID channel).
- A process-wide file lock is acquired **before** any reservation write.
- Gate checks (idempotency, cooldown, quota, spend) **validate first**, then
  persist all reservations atomically — any rejection consumes nothing.
- Min-success is forced equal to the configured worker count.
- `--skip-evidence` and `--evidence-file` are refused (shared mode must not
  accept free evidence overrides).
- Topic is control-char stripped and hard-capped before model I/O.

## Integration paths

### 1. Standalone CLI

```bash
printf '%s' 'topic' | python3 examples/last30days-agent/scripts/last30days.py --topic-stdin
```

### 2. Persona pack (Desktop Install Pack)

```bash
buzz pack validate ./examples/last30days-agent
buzz pack inspect ./examples/last30days-agent
# Desktop: Install Pack → point at this directory
```

The persona (`agents/last30days.persona.md`) advertises `/last30days` and
instructs the runtime to invoke `scripts/last30days.py` with the topic from ACP
block 0. This relies on existing ACP slash pass-through (#919); it does **not**
patch core and does **not** open a direct relay watcher.

### 3. External evidence harness

```bash
export LAST30DAYS_EVIDENCE_CMD='["my-research-tool","--topic","{topic}","--days","{days}"]'
printf '%s' 'topic' | python3 examples/last30days-agent/scripts/last30days.py --topic-stdin
```

Any tool that prints a markdown brief to stdout works. The topic is one opaque
argv element (`shell=False`). API keys are never exported into the evidence
child process.

## ACP slash contract (#919)

| Rule | Detail |
|------|--------|
| Trigger | Single **non-cancelled** slash event only |
| ACP block 0 | Bare command (`/last30days <topic>`) — only source of the topic |
| ACP block 1 | Wrapped current Buzz context — channel, thread, requester |
| Non-triggers | Message batches, cancel carryover, plain messages without slash |

## Thread publication

After the swarm succeeds (or fails with a public error), publish with the Buzz
CLI. **Both** are required before claiming delivery:

1. Process exit code `0`
2. Stdout JSON includes a signed `event_id`

## Manual Buzz smoke test

1. Export `OPENAI_API_KEY` (and optional base URL / model) in the agent runtime
   environment — not in persona or skill files.
2. Install the pack (`buzz pack validate` + Desktop Install Pack), or shell the
   CLI from a connector that receives #919 pass-through.
3. In a channel, send: `/last30days Buzz multi-agent collaboration`
4. Expect: short acknowledgement, then a threaded brief only after 10 usable
   workers + synthesis succeed (or a sanitized public error).
5. Confirm run dir modes are `0700` / files `0600`, and `receipt.json` contains
   no API key material.

## Security notes

- **Env-only secrets.** No dotenv loader and no secret path overrides.
- **Base URL trust boundary.** Changing `OPENAI_BASE_URL` /
  `LAST30DAYS_BASE_URL` sends the adopter API key to that host. Only set a
  custom base when the operator explicitly trusts the endpoint.
- **No shell injection.** Topics enter via `--topic-stdin` / `--topic-file`.
  Evidence commands are JSON argv arrays with `shell=False`; untrusted topic
  text is never interpolated into a shell string. Templates whose argv[0] is a
  shell interpreter (`sh`/`bash`/`zsh`/`dash`/`cmd`/`powershell`/…) and whose
  `-c`/`-Command` body embeds `{topic}`/`{days}`/`{out_dir}` are rejected so a
  plausible operator template cannot turn chat text into shell code.
- **Transactional gates.** Shared-mode rejections consume no idempotency,
  quota, or spend reservation. All reservations live in one consolidated
  `gate-state.json`, persisted via temp-file + fsync + `os.replace` under the
  concurrency lock. Unparseable state fails CLOSED (not treated as empty).
- **Content-only deliverables.** Empty/`length`/reasoning-only model replies are
  failures; retries escalate token budget with real headroom.
- **Sanitized public errors.** Keys, Bearer tokens, common secret shapes, and
  absolute filesystem paths are redacted before stderr / receipt serialization.
- **Private artifacts.** Run directories `0700`, files `0600` on Unix.
- **Minimal receipts.** `receipt.json` is metadata only (model, provider,
  tokens, cost, status, timings). Topic, brief, paths, and gate identity live
  in private purpose-specific files.

## Offline tests

```bash
python3 examples/last30days-agent/scripts/test_last30days.py
```

Coverage includes: identity validation, topic cap, content usability,
min-success=10, spend reservation (spent+reserved+this ≤ ceiling),
lock-before-reserve, idempotency, redaction, home-path hygiene, and a full
mocked 10+1 happy path.

## Precedent

Pack shape from [`meadow-core`](../meadow-core/); runnable-reference style from
[`countdown-bot`](../countdown-bot/) (#516).
