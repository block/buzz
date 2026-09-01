---
name: last30days
description: "Run the provider-agnostic Last30Days multi-worker swarm and publish a threaded brief via Buzz CLI."
---

# Last30Days skill

## Orchestrator

**Never** put the topic in a shell-quoted string (`python3 … "<topic>"`). Topics come from untrusted chat and may contain `$()`, backticks, or `;`. Use stdin or a topic file so the topic is an opaque byte string.

```bash
# From pack root (or pass absolute path after install)
# Preferred: opaque topic via stdin
printf '%s' "$TOPIC" | python3 scripts/last30days.py --topic-stdin --emit brief

# Or: opaque topic via file
python3 scripts/last30days.py --topic-file /path/to/topic.txt --emit brief

# Shared/channel mode (gates ON)
printf '%s' "$TOPIC" | python3 scripts/last30days.py \
  --topic-stdin \
  --enforce-gates \
  --event-id <64-hex> \
  --requester <64-hex> \
  --channel <uuid> \
  --emit brief

# Offline tests (no network, no key)
python3 scripts/test_last30days.py
```

Exit codes from `scripts/last30days.py`: `0` pass, `1` user/config error, `2` swarm failed (min-success or synthesis).

## ACP slash contract (#919)

| Rule | Detail |
|------|--------|
| Trigger | Single **non-cancelled** slash event only |
| ACP block 0 | Bare command (`/last30days <topic>`) — **only** source of the topic |
| ACP block 1 | Wrapped current Buzz context — channel, thread, requester; not part of topic |
| Non-triggers | Message batches, cancel carryover, plain messages without slash |

Do not reimplement slash routing. Rely on existing ACP pass-through.

## Thread publication (required proof)

After the swarm succeeds (or fails with a public error), publish to the originating thread with the Buzz CLI.

**Success criteria — both required:**

1. Process **exit code 0**
2. Stdout JSON includes a signed **`event_id`** (and typically `accepted: true`)

Example pattern:

```bash
printf '%s\n' "$BRIEF" | buzz messages send \
  --channel "$CHANNEL_UUID" \
  --content - \
  --reply-to "$THREAD_EVENT_ID"
# Verify: exit 0 AND parse event_id from JSON stdout
```

If exit ≠ 0 or `event_id` is missing, treat publication as **failed** — retry once with a shorter body or post a sanitized blocker. Never claim the brief was delivered without that proof.

For @mentions that must notify, pass `--mention <hex-or-npub>` and confirm `mention_pubkeys` in the success JSON.

## Trust: base URL and API key

- Default base URL is OpenRouter-compatible: `https://openrouter.ai/api/v1`
- Setting `OPENAI_BASE_URL` / `LAST30DAYS_BASE_URL` to any other host **sends the adopter API key to that host**
- Operators must explicitly trust the configured endpoint before changing the base URL
- Never write the key into prompts, receipts, channel messages, or evidence child env

## Evidence

Optional. `LAST30DAYS_EVIDENCE_CMD` must be a **JSON argv array** (not a shell string), executed with `shell=False`. Placeholders `{topic}`, `{days}`, `{out_dir}` are substituted as opaque argv elements.

```bash
export LAST30DAYS_EVIDENCE_CMD='["my-research-tool","--topic","{topic}","--days","{days}"]'
```

Under `--enforce-gates`, `--skip-evidence` and `--evidence-file` are refused.

## Artifact safety

- Run dirs mode `0700`, files `0600`
- `receipt.json`: metadata only (model/provider/tokens/cost/status/timings) — no topic, brief, paths, or gate identity
- `run-context.json` + `brief.md` / `worker-*.md`: private purpose-specific artifacts
- Content-only worker success (no reasoning-field fallback)
