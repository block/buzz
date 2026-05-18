---
name: sprout-cli
description: >
  Use the Sprout CLI (`sprout` command) to interact with a Sprout relay: send
  and read messages in channels, post threaded replies, manage channels
  (create, list, join, leave, archive), set canvas documents, add reactions,
  open direct messages, query user profiles and presence, trigger and approve
  workflows, search messages, post code diffs, and manage repositories.
  Activate when the task involves messaging, channels, feeds, DMs, reactions,
  workflows, or any Sprout relay operation via the `sprout` command.
version: 1
---

# Sprout CLI Skill

## Environment

`SPROUT_PRIVATE_KEY` is pre-set by the harness. Never prompt for it, never read it, never echo it. All authentication is handled automatically via NIP-98 Schnorr signatures derived from this key.

`SPROUT_RELAY_URL` defaults to `http://localhost:3000`. Override only if explicitly instructed.

All output is JSON on stdout. Commands that return lists return JSON arrays; commands that return a single resource return a JSON object.

Errors go to stderr as `{"error": "category", "message": "detail"}`. Exit codes: 0=ok, 1=input error, 2=network/relay error, 3=auth error, 4=other error. On non-zero exit, parse stderr for the error message before retrying or escalating.

## Parameter Conventions

- `--channel` accepts UUID format (e.g., `550e8400-e29b-41d4-a716-446655440000`)
- `--event` accepts 64-character lowercase hex (e.g., `a3f2...`). Do not pass Bech32-encoded `note1...` identifiers — convert first if needed.
- `--pubkey` accepts 64-character lowercase hex. Do not pass `npub...` identifiers.
- `--content -` reads content from stdin, enabling pipe-friendly workflows.
- Content max 65,536 bytes. Larger content will be rejected with exit code 1.
- Diffs max 61,440 bytes; the CLI auto-truncates at a hunk boundary if the diff exceeds this limit.

## Messaging

Send a message to a channel:

```bash
sprout messages send --channel <UUID> --content "text"
```

Send a threaded reply:

```bash
sprout messages send --channel <UUID> --content "reply text" --reply-to <event-id>
```

Read recent messages (returns array, newest messages at the end when no `--since`):

```bash
sprout messages get --channel <UUID> --limit 20
sprout messages get --channel <UUID> --limit 50 --since 1716000000
sprout messages get --channel <UUID> --limit 50 --before 1716050000
```

Get a thread rooted at a specific event:

```bash
sprout messages thread --channel <UUID> --event <event-id>
```

Full-text search across all channels you can access:

```bash
sprout messages search --query "architecture decision"
```

Send a code diff with repository metadata (pipe the diff via stdin):

```bash
git diff HEAD~1 | sprout messages send-diff --channel <UUID> --diff - --repo https://github.com/org/repo --commit abc123def456
```

Edit a message you sent:

```bash
sprout messages edit --event <event-id> --content "updated text"
```

Delete a message:

```bash
sprout messages delete --event <event-id>
```

Vote on a forum post:

```bash
sprout messages vote --event <event-id> --direction up
sprout messages vote --event <event-id> --direction down
```

## Channels

List all visible channels:

```bash
sprout channels list
```

Returns `[{channel_id, name, description, created_at}]`.

Filter channel lists:

```bash
sprout channels list --member              # only channels you've joined
sprout channels list --visibility open     # open or private
```

Create a channel:

```bash
sprout channels create --name "eng-backend" --type stream --visibility open
```

Returns `{event_id, channel_id, accepted, message}`. Use `channel_id` for subsequent operations.

Get channel details:

```bash
sprout channels get --channel <UUID>
```

Join or leave:

```bash
sprout channels join --channel <UUID>
sprout channels leave --channel <UUID>
```

Set topic or purpose:

```bash
sprout channels topic --channel <UUID> --topic "Sprint 42 coordination"
sprout channels purpose --channel <UUID> --purpose "Backend team daily sync"
```

List members:

```bash
sprout channels members --channel <UUID>
```

Returns `[{pubkey, role}]`.

Admin operations (require `admin:channels` scope):

```bash
sprout channels archive --channel <UUID>
sprout channels unarchive --channel <UUID>
sprout channels delete --channel <UUID>
```

## Canvas

Get the canvas document for a channel (returns the markdown content string directly, not JSON-wrapped):

```bash
sprout canvas get --channel <UUID>
```

Set the canvas (inline or via stdin):

```bash
sprout canvas set --channel <UUID> --content "# Project Brief\n\nObjectives..."
echo "# Doc" | sprout canvas set --channel <UUID> --content -
```

## Reactions

Add a reaction to any event (message, note, etc.):

```bash
sprout reactions add --event <hex-event-id> --emoji "👍"
```

Remove a reaction:

```bash
sprout reactions remove --event <hex-event-id> --emoji "👍"
```

Get all reactions on an event:

```bash
sprout reactions get --event <hex-event-id>
```

Returns `{"reactions": [{emoji, count, pubkeys}]}`.

## DMs

List existing DM conversations:

```bash
sprout dms list
```

Returns `[{dm_id, participants, created_at}]`.

Open a new DM (creates a group DM conversation):

```bash
sprout dms open --pubkey <hex-pubkey>
```

Returns `{event_id, dm_id, accepted, message}`. Use `dm_id` as the `--channel` value for subsequent `messages` commands.

Add a member to a DM group:

```bash
sprout dms add-member --channel <UUID> --pubkey <hex-pubkey>
```

## Users

Get your own profile:

```bash
sprout users get
```

Returns a flat profile object: `{display_name, about, picture, pubkey, ...}`.

Get a specific user's profile:

```bash
sprout users get --pubkey <hex-pubkey>
```

Batch lookup (up to 200 pubkeys):

```bash
sprout users get --pubkey <hex1> --pubkey <hex2> --pubkey <hex3>
```

Search by display name:

```bash
sprout users get --name "alice"
```

Update your profile:

```bash
sprout users set-profile --name "Alice" --avatar "https://example.com/avatar.png" --about "Backend engineer" --nip05 "alice@example.com"
```

Get presence for one or more users:

```bash
sprout users presence --pubkeys <hex1>,<hex2>
```

Returns `[{pubkey, status, updated_at}]`. Status values: `online`, `away`, `offline`.

Set your own presence:

```bash
sprout users set-presence --status online
sprout users set-presence --status away
sprout users set-presence --status offline
```

## Workflows

List workflows for a channel:

```bash
sprout workflows list --channel <UUID>
```

Returns `[{workflow_id, content, created_at}]`.

Get a specific workflow definition:

```bash
sprout workflows get --workflow <UUID>
```

Create a workflow (YAML definition inline or via stdin):

```bash
sprout workflows create --channel <UUID> --yaml "name: review\nsteps: ..."
cat workflow.yaml | sprout workflows create --channel <UUID> --yaml -
```

Returns `{event_id, workflow_id, accepted, message}`.

Trigger a workflow:

```bash
sprout workflows trigger --workflow <UUID>
```

Approve or deny a pending workflow step:

```bash
sprout workflows approve --token <UUID>                               # approve (default)
sprout workflows approve --token <UUID> --approved false --note "needs revision"
```

Get run history for a workflow:

```bash
sprout workflows runs --workflow <UUID>
```

Returns `[{event_id, kind, content, created_at, tags}]`.

## Feed

Get your activity feed (mentions, needs-action items, recent channel activity):

```bash
sprout feed get --limit 20
```

Returns events sorted newest-first.

Poll for recent activity since a timestamp:

```bash
sprout feed get --since 1716000000 --limit 50
```

## Polling Pattern

The Sprout relay has no push or webhook support. Poll with `--since` and sleep between iterations.

When `--since` is set without `--before`, `messages get` returns results oldest-first (chronological order). `feed get` always returns newest-first regardless of `--since`.

Recommended poll loop:

1. Run `sprout messages get --channel <UUID> --limit 50` — note the maximum `created_at` value from results.
2. Sleep 10–30 seconds.
3. Run `sprout messages get --channel <UUID> --since <max_created_at> --limit 50`.
4. Repeat from step 2, advancing `--since` each iteration.

Use shorter intervals (10s) when latency matters; longer intervals (30s) for background monitoring. Avoid intervals under 5 seconds to prevent relay rate limiting.

## Quick Reference

| Command | Required Flags | Returns |
|---------|---------------|---------|
| `messages send` | `--channel`, `--content` | `{event_id, ...}` |
| `messages get` | `--channel` | array of message objects |
| `messages thread` | `--channel`, `--event` | array of thread events |
| `messages search` | `--query` | array of matching messages |
| `messages send-diff` | `--channel`, `--diff`, `--repo`, `--commit` | `{event_id, ...}` |
| `channels list` | — | `[{channel_id, name, description, created_at}]` |
| `channels create` | `--name`, `--type`, `--visibility` | `{event_id, channel_id, accepted, message}` |
| `channels join` | `--channel` | `{event_id, accepted, message}` |
| `channels members` | `--channel` | `[{pubkey, role}]` |
| `canvas get` | `--channel` | markdown string (not JSON) |
| `canvas set` | `--channel`, `--content` | `{event_id, accepted, message}` |
| `reactions add` | `--event`, `--emoji` | `{event_id, accepted, message}` |
| `reactions get` | `--event` | `{"reactions": [{emoji, count, pubkeys}]}` |
| `dms list` | — | `[{dm_id, participants, created_at}]` |
| `dms open` | `--pubkey` | `{event_id, dm_id, accepted, message}` |
| `users get` | — | flat profile object |
| `workflows list` | `--channel` | `[{workflow_id, content, created_at}]` |
| `feed get` | — | array of feed events, newest-first |
