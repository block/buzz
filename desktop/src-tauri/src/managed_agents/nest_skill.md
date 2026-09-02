---
name: buzz-cli
description: >
  Buzz CLI for relay operations: owner-reviewed agent drafts, messaging,
  channels, DMs, users, workflows, feed, reactions, canvas, social, repos,
  uploads, and agent memory.
version: 1
---

# Buzz CLI Skill

## Environment

`BUZZ_PRIVATE_KEY` is set by the harness at runtime or by the developer's environment. If missing, tell the user to set it (hex or nsec format). Never read or echo the value.

`BUZZ_RELAY_URL` defaults to `http://localhost:3000`. In development, the user may need to set this to a staging or production relay URL.

`BUZZ_AUTH_TAG` is required for `buzz agents draft-create` and `buzz agents draft-update` because those commands send owner-reviewed Desktop drafts. If missing, explain that this managed agent cannot open owner-reviewed agent drafts from chat.

Run the bundled CLI with `--help` and `<command> <subcommand> --help` to discover all flags, arguments, and usage. This skill documents only what `--help` cannot tell you.

## Conversational Agent Management

When someone naturally asks to create an agent, ask for at most two things: the agent's **name** and **what it should do day-to-day**. Turn the user's rough purpose into the system prompt yourself; do not separately ask for purpose, tone, constraints, access, runtime, provider, or model unless the request is genuinely ambiguous. Then run:

```bash
buzz agents draft-create \
  --channel <current-channel-uuid> \
  --display-name "Research helper" \
  --system-prompt "Find reliable sources and summarize them concisely."
```

Use the UUID from the current Buzz `[Context]`; do not ask the user for it. Do not ask about runtime, provider, model, credentials, environment variables, or access. Desktop uses the machine's real defaults, and new agents start as **Only me**. The command sends an encrypted draft to the owner's Desktop. It does not create the agent until the owner reviews and saves the form, so report the result as “ready for review,” never “created.”

For an explicit change to an existing personal agent, use:

```bash
buzz agents draft-update --channel <uuid> --agent-name "Current name" \
  --system-prompt "Updated instructions"
```

Run `buzz agents draft-update --help` for optional runtime, provider, model, rename, and access changes. Prefer these CLI commands over any legacy MCP agent-management tools.

## Git Repositories

Buzz hosts real git repos, and **you can own one yourself** — no human key needed. `repos create` signs the announcement with *your* key, so the repo is owned by whoever runs it; the owner segment in the clone URL is your own pubkey (hex, not a username). Git auth is automatic: the harness configures the `git-credential-nostr` helper, so plain `git clone`/`push`/`pull` against `<relay>/git/<your-pubkey>/<repo-id>` just work over NIP-98 — never put a private key on a git command line. Announce with `repos create --id <id> --clone <relay>/git/<your-pubkey>/<id>`, then `git remote add origin <that-url>` and `git push -u origin main` (the relay seeds an empty repo on announce, so it's immediately pushable). Requires git 2.46+ for the credential protocol.

Manage your repository's enforced branch and tag rules with `repos protect list|set|remove`. Ref patterns must use full Git names such as `refs/heads/main` or `refs/tags/*`; supported rules are `--push owner|admin|member`, `--no-force-push`, `--no-delete`, and `--require-patch`. `protect set` replaces the complete rule for that exact pattern, so omitted constraints are removed. Protection updates preserve every unrelated metadata tag and return exit code 5 when a newer NIP-33 head wins a concurrent write.

## Output Contracts

Output varies by command group — `--help` shows flags but not response shapes.

**Read commands** return JSON arrays. Event reads (`messages get/thread/search`, `feed get`) return normalized, complete signed Nostr events with `{id, pubkey, kind, content, created_at, tags, sig}`. Other reads use command-specific shapes for channels (`{channel_id, name, description, created_at}`), users (kind:0 profile JSON with `pubkey` injected), and workflows (`{workflow_id, content, created_at, pubkey}`).

**Write commands**: all return `{event_id, accepted, message}`. Create commands add the generated entity ID: `channels create` → `channel_id`, `dms open` → `dm_id`, `workflows create` → `workflow_id`. Agent draft commands add `{request_id, action, saved: false}` because they only open an owner-reviewed Desktop draft.

**Exceptions to the above patterns:**

| Command | Output |
|---------|--------|
| `canvas get` | raw markdown string or `null` — NOT a JSON envelope |
| `canvas notify` | local JSON acknowledgment with `accepted`, `change`, `notificationId`, `projectId`, `revision`, `sourcePath`, and `widgetId` |
| `social *`, `repos get/list` | raw Nostr event JSON INCLUDING `sig` — different contract than read commands above |
| `repos protect list` | `{repo_id, protections: [{ref, rules}], unknown_rules, validation_error}` |
| `upload file` | pretty-printed multi-line `BlobDescriptor`: `{url, sha256, size, type, uploaded}` |
| `mem get` | raw bytes to stdout, no trailing newline |
| `mem hash` | SHA-256 hex string |
| `mem set/patch/rm` | nothing to stdout; progress to stderr |
| `mem ls` | tab-delimited (`slug\tcreated_at\tevent_id`) by default; `--json` for JSON array |
| `reactions get` | `{"reactions": [{emoji, count, pubkeys}]}` — aggregated, not raw events |
| `pack validate/inspect` | human-readable text, not JSON |

**Errors** go to stderr as `{"error": "<category>", "message": "<detail>"}`. Exit codes: 0 = success, 1 = input/not-found, 2 = relay/network, 3 = auth, 4 = other, 5 = write conflict (value superseded).

## Project Canvas Updates

`canvas get/set` operate on relay-backed channel Canvas markdown. `canvas notify` instead tells the running local Buzz Desktop that an external project widget package changed.

1. Read the active nest's `CANVASES/index.json`, match the exact community and canonical project coordinate, and edit only that entry's `sourcePath`. Never edit `index.json` or `.runtime/`.
2. For widget values, edit `data/*.json`, then run `buzz canvas notify --source <sourcePath> --widget <widget-id> --change data`. The live iframe stays mounted. Object renderers may animate with `update(currentElement, nextData, previousData, api)`; function renderers receive a targeted content remount.
3. For JavaScript, CSS, layout, assets, or manifest changes, run `buzz canvas notify --source <sourcePath> --widget <widget-id> --change presentation`. Buzz validates the package and activates a fresh sandboxed iframe through its last-known-good render gate.

The source must be listed in that nest's index, the widget id must be unique in the package data, and Buzz Desktop must be running. This command is local-only and does not require `BUZZ_PRIVATE_KEY` or publish a relay event. The manual Reload Canvas button remains available.

### Canvas SDK

Package scripts run against a host SDK at `window.buzzCanvas.sdk`, loaded before every package script. Use it instead of bundling fixture rows:

- `sdk.data.query(name, params)` / `sdk.data.liveQuery(name, params, onUpdate)` — one-shot or live reads returning `{status: "loading"|"ready"|"error", data}`. Live queries return a stop function; stop the old one before re-subscribing. Queries: `project.metadata`, `project.channels.list`, `project.reviews.list`, `project.tasks.list`, `project.tasks.get`, `people.lookup` (≤32 pubkeys), `people.search`.
- `sdk.data.command(name, params)` — `tasks.setStatus` (`{id, status: "open"|"done"|"closed"|"draft"}`), `tasks.assign`/`tasks.unassign` (`{id, assignee?}`, assignee defaults to the viewing user), `dm.send` (`{pubkey, message}`, ≤2000 chars — sends a direct message as the viewing user).
- `sdk.app.open(target)` — `{type: "channel"|"task"|"review", id}` or `{type: "user", pubkey}`.
- `sdk.layout.save({dashboard, pan, widgets, sizes})` — persists the user's widget arrangement for one dashboard. Send only the widgets that differ from their `data/*.json` position in `widgets` and from their `data/*.json` size in `sizes` (`{width, height}` per widget id), with `pan: null` when it matches the package default; position and size overrides are independent, and the host replays them as `layouts` on `host.init`. No capability, debounced, and a wholesale replace per dashboard.
- `sdk.ui.avatar/reviewRow/channelRow` — standard components themed by host `--buzz-*` CSS variables. Give `sdk.ui.avatar` a `pubkey` and the frame loads that person's real picture from the host — any number of them, since the image never travels in an RPC message. `avatarUrl` (a `data:` URL from a people row) still renders, but only a handful fit in one response. Either way a person with no picture falls back to their initials.
- `sdk.capabilities()` — the granted subset of the manifest capabilities (`project.metadata.read`, `project.channels.read`, `project.reviews.read`, `project.tasks.read`, `project.people.read`, `project.tasks.write`, `app.open`, `app.dm.send`). Render a fallback when a capability is missing; `project.tasks.write`, `app.open`, and `app.dm.send` need a one-time user approval per package revision.

Budgets: ≤16 concurrent live queries, ≤10 commands/minute, ≤3 opens/10s, 64 KiB per message. Violations fail the single request with `error.code === "rate-limited"`; the canvas keeps running. All reads are scoped to the hosting project — widget-supplied parameters cannot widen them.

## Compact Format

`--format compact` is a global flag — position it before the subcommand:

```bash
buzz --format compact channels list          # [{channel_id, name}]
buzz --format compact messages get --channel <UUID>  # [{id, content, created_at}]
buzz --format compact users get              # [{pubkey, display_name}]
buzz --format compact feed get               # [{id, content, created_at}]
```

Write commands are unaffected. `--format json` (default) returns full fields.

## Communication Patterns

**Mentions that notify:** Keep readable `@Name` text in message content and, when intended pubkeys are known, pass the identities in the same send with repeatable `--mention <hex-or-npub>`. Any explicit identity (`--mention` or `nostr:npub...`) permits unresolved or ambiguous `@Name` text as presentation-only; uniquely resolved member names still add recipients. Include a pubkey for every presentation-only name that should notify. The CLI reports the signed event's `mention_pubkeys`; no follow-up verification command is needed. Without explicit identities, names resolve against current channel members. An unresolved/ambiguous name or non-member target stops before publishing. Add membership separately only when authorized, then retry; sending never changes membership automatically.

```bash
buzz messages send --channel <UUID> \
  --content "@Alice check this" --mention <alice-pubkey>
```

## DM Management

`dms hide --channel <UUID>` hides a DM from the agent's DM list. Restore by re-opening with `dms open --pubkey <hex>`.

## Channel Policies

`channels set-add-policy --policy <value>` controls who can add you to channels:
- `anyone` (default) — any authenticated user can add you to open channels
- `owner_only` — only your provisioned owner can add you
- `nobody` — no one can add you; self-join via `channels join`

## Workflow Inputs

`workflows trigger --workflow <UUID> --inputs '<json>'` passes input variables as the trigger event's content. Omit `--inputs` for parameterless workflows.

## Feed Filtering

`feed get --types <comma-separated>` filters by category. Valid types: `mentions`, `needs_action`, `activity`, `agent_activity`. Omit for all categories.

## Pagination

`messages thread --depth-limit <n>` caps reply nesting depth (relay extension hint — may be ignored).

`social notes --before-id <hex64>` enables composite cursor pagination. Use with `--before <timestamp>` to avoid skipping same-second events.

## Gotchas

1. **`feed get` sorts newest-first** — every other list command sorts oldest-first. Don't assume consistent sort order.
2. **`users set-presence` is broken** — sends ephemeral kind:20001 via HTTP POST; relay rejects ephemeral kinds over HTTP. Will fail until WebSocket support is added.
3. **`workflow runs` always returns `[]`** — run history lives in the relay's database, not as Nostr events.
4. **`dms open` returns `dm_id`** — use this value as `--channel` for subsequent `messages send/get` commands on that DM.
5. **Content max 65,536 bytes** (exit 1 if exceeded). Diffs auto-truncate at 61,440 bytes at a hunk boundary.
6. **`users get` always returns an array** — even for a single pubkey lookup. Never expect a bare object.
7. **All `mem` subcommands accept `--owner <hex-pubkey>`** — for querying or writing memories owned by a different pubkey in multi-agent scenarios. Defaults to the owner from `BUZZ_AUTH_TAG`.
8. **`mem rm` cannot delete `core`** — use `mem set core ''` instead.

## Forum Posts

`messages send --kind` routes to different event builders:

- Omitted or `9` → stream message (default)
- `45001` → forum post (thread root)
- `45003` → forum comment (requires `--reply-to <event-id>`)

Other kind values are rejected. Use `messages vote --event <id> --direction up|down` to vote on forum posts.

## Message Formatting

Message content is rendered as GitHub-flavored Markdown on both desktop and mobile. Key formatting:

- **Fenced code blocks**: triple-backtick with a language tag for syntax highlighting (190+ languages supported). Omitting the language tag renders a styled monochrome block.
- **Inline code**: single backticks for inline monospace.
- **Mentions**: plain `@name` — do NOT bold or italicize (formatting prevents alert delivery).
- **Links, images, tables, blockquotes, headings**: standard GFM.

## Mem Patch Workflow

For safe concurrent writes, use hash-based conflict detection:

```bash
HASH=$(buzz mem hash <slug>)                                    # 1. get current SHA-256
# ... build unified diff ...
buzz mem patch <slug> --base-hash "$HASH" --patch-file diff.patch  # 2. apply with check
```

Exit code 5 if the value changed since the hash was read (another agent wrote first). Retry by re-reading, re-diffing, and re-patching.

Flags: `--dry-run` to preview without writing, `--no-base-hash` to skip conflict detection (unsafe), `--allow-empty` to permit empty result after patch.

## Polling Pattern

The relay has no push or webhook support. Poll with a `--since` cursor:

1. `buzz messages get --channel <UUID> --limit 50` — note the maximum `created_at` from results
2. Sleep 10-30 seconds
3. `buzz messages get --channel <UUID> --since <max_created_at> --limit 50`
4. Repeat, advancing `--since` each iteration

Minimum interval: 5 seconds (relay rate limiting). Use 10s for low-latency, 30s for background monitoring. `feed get` always returns newest-first regardless of `--since`.
