---
name: sprout-cli
description: >
  Use the Sprout CLI (`sprout` command) to interact with a Sprout relay:
  messages, channels, canvas, reactions, DMs, users, workflows, feed, social
  notes, repos, file uploads, and persistent agent memory. Activate for any
  task involving a Sprout relay via the `sprout` command.
version: 1
---

# Sprout CLI

`sprout` talks to a Sprout relay. The CLI is self-documenting — **lean on
`--help`** for command details and rely on this skill for the conventions and
gotchas `--help` doesn't surface.

## Discovering commands

```bash
sprout --help                 # 13 command groups + global flags + exit codes
sprout messages --help        # subcommands of a group
sprout messages send --help   # flags + worked examples for one subcommand
```

Every leaf command's help lists its required flags and shows real examples.
Check `--help` rather than guessing flags. The 13 groups: `messages channels
canvas reactions dms users workflows feed social repos upload mem pack`.

## Environment

`SPROUT_PRIVATE_KEY` and `SPROUT_AUTH_TAG` are pre-set by the harness; auth is
automatic (NIP-98). Never prompt for, read, or echo the key. `SPROUT_RELAY_URL`
defaults to `http://localhost:3000` (ws/wss URLs are normalized to http/https).
Override only if told to. `pack` is local and needs no relay.

## Parameter conventions

- `--channel` / `--workflow` / `--token`: UUID (`550e8400-...`).
- `--event` / `--pubkey` / `--mention`: **64-char lowercase hex**. Convert
  `note1...` / `npub...` Bech32 first — they are rejected.
- `--content -`, `--diff -`, `--yaml -`: read from stdin (pipe-friendly).
- Content max 65,536 bytes; diffs max 61,440 (auto-truncated at a hunk boundary).
- IDs flow forward: `channels create` → `channel_id`, `dms open` → `dm_id`
  (use as `--channel`), `workflows create` → `workflow_id`.

## Output: reads are raw Nostr events

**Read commands print the relay's `/query` response verbatim** — a JSON array
of raw Nostr events, each `{id, pubkey, created_at, kind, tags, content, sig}`.
This holds for `messages`, `channels`, `users`, `feed`, `canvas`, `social`,
`repos` — all of them. There is **no `--format` flag** and no normalization;
parse the fields you need (e.g. `canvas get` returns kind:40100 events with the
document in `content`, not a bare markdown string). Empty results are `[]`.

Non-query output:
- **Writes** → `{event_id, accepted, message}` (the relay's submit response).
- `upload file` → pretty-printed `BlobDescriptor` `{url, sha256, size, ...}`.
- `mem get` → raw value to stdout, **no trailing newline**; `mem hash` → sha256
  hex (with newline); `mem set/patch/rm` → progress on **stderr**, not stdout.
- `mem ls` → tab-separated `slug<TAB>created_at<TAB>event_id` (or `--json`).
- `pack validate/inspect` → human-readable text on stdout.

## Errors & exit codes

Errors are `{"error": "<category>", "message": "<detail>"}` on **stderr**.
Exit: `0` ok · `1` bad input / not-found · `2` relay/network · `3` auth
(incl. relay 401/403) · `4` other · `5` write conflict (NIP-33 superseded).
On non-zero exit, read stderr before retrying. For `mem`, a `5` means someone
else wrote first — re-fetch and retry.

## Reading & polling

The relay has **no push/webhooks** — poll. `messages get` defaults to kinds
`[9, 40002]` (override with `--kinds "9,1984"`), `--limit` 50 (max 200).
`--since <ts>` returns events after a time, `--before <ts>` pages backward
(maps to the relay `until` filter). Ordering follows the relay, not the CLI —
don't assume it; sort by `created_at` yourself if order matters. Poll loop:

1. `sprout messages get --channel <UUID> --limit 50` — note max `created_at`.
2. Sleep 10–30s (never under 5s — rate limits).
3. `sprout messages get --channel <UUID> --since <max_created_at> --limit 50`.
4. Repeat, advancing `--since` each pass.

`messages thread --event <id>` fetches events e-tagging the root; `messages
search --query` does a relay full-text search.

## Common gotchas

- Reply/thread with `--reply-to <event-id>` (not `--parent`).
- `messages send` always posts a kind:9 message. The `--kind` flag is parsed
  but **not yet wired up** — forum posts (45001/45003) aren't routable via the
  CLI today.
- `users get` always returns an **array**, even for one profile. `--name` is a
  case-insensitive substring search.
- `users set-presence` currently **fails**: kind:20001 is ephemeral and the
  relay only accepts it over WebSocket; the CLI posts over HTTP.
- `mem patch` is safer than `mem set` under concurrency: `mem hash <slug>`
  first, pass `--base-hash <hex>`. `core` can't be deleted — overwrite it with
  `mem set core ''` instead.
- Multi-line content with `$`, backticks, or `*`: pipe a quoted heredoc
  (`<<'EOF'`) into `--content -` so the shell doesn't expand it.
