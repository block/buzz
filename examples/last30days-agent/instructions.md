# Last30Days pack instructions

## Scope

This pack is an **examples-only** reference. It exercises the existing ACP slash pass-through ([#919](https://github.com/block/buzz/pull/919)). Discovery/autocomplete remain owned by [#2528](https://github.com/block/buzz/issues/2528) / [#3537](https://github.com/block/buzz/pull/3537). Do not patch core relay or Desktop command routing.

## Command surface

- Advertise and handle: `/last30days <topic>`
- Default swarm: **10** independent workers + **1** synthesis
- Default model slug: `deepseek/deepseek-v4-pro` (adopter supplies key and may change model/base URL)

## Safety

- Secrets from process environment only — no dotenv / secret-file discovery
- Changing `OPENAI_BASE_URL` sends the API key to that host; require explicit operator trust
- Pass topics via `--topic-stdin` or `--topic-file` only — never shell-quote untrusted topic text
- Evidence command is a JSON argv array executed with `shell=False` (no shell templates)
- Public errors sanitized; artifacts owner-only (`0700` / `0600`); `receipt.json` is metadata-only
- Thread posts only after `buzz messages send` exit 0 **and** signed `event_id` JSON

## Communication

- Short in-thread pickup, then one deliverable or sanitized blocker
- Prefer the orchestrator brief shape over free-form essays
