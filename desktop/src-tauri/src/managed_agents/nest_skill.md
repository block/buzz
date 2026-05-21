---
name: sprout-cli
description: >
  Use the Sprout CLI (`sprout` command) to interact with a Sprout relay: send
  and read messages, manage channels, set canvas documents, add reactions,
  open DMs, query user profiles, trigger workflows, search messages, post code
  diffs, manage repositories, publish social notes, upload files, and manage
  persistent agent memory. Activate when the task involves messaging, channels,
  feeds, DMs, reactions, workflows, social notes, repos, uploads, agent memory,
  or any Sprout relay operation via the `sprout` command.
version: 1
---

# Sprout CLI Skill

## Environment

`SPROUT_PRIVATE_KEY` is pre-set by the harness. Never prompt for it, never read it, never echo it. All authentication is handled automatically via NIP-98 Schnorr signatures derived from this key.

`SPROUT_RELAY_URL` defaults to `http://localhost:3000`. Override only if explicitly instructed.

All output is JSON on stdout unless noted otherwise. Commands that return lists return JSON arrays; commands that return a single resource return a JSON object. Exceptions: `canvas get` returns a raw markdown string (not JSON); `mem get`/`mem hash` write raw values to stdout; `pack inspect` outputs human-readable text; `social` and `repos` commands return raw Nostr event JSON (includes `sig` and relay-specific fields — not the normalized format used by `messages`, `channels`, etc.).

Errors go to stderr as `{"error": "<category>", "message": "<detail>"}`. Category values: `user_error` (exit 1), `relay_error` / `network_error` (exit 2), `auth_error` / `key_error` (exit 3), `error` (exit 4), `conflict` (exit 5 — write conflict, relay rejected as superseded). On non-zero exit, parse stderr for the error message before retrying or escalating.

## Parameter Conventions

- `--channel` accepts UUID format (e.g., `550e8400-e29b-41d4-a716-446655440000`)
- `--event` accepts 64-character lowercase hex (e.g., `a3f2...`). Do not pass Bech32-encoded `note1...` identifiers — convert first if needed.
- `--pubkey` accepts 64-character lowercase hex. Do not pass `npub...` identifiers.
- `--content -` reads content from stdin, enabling pipe-friendly workflows.
- Content max 65,536 bytes. Larger content will be rejected with exit code 1.
- Diffs max 61,440 bytes; the CLI auto-truncates at a hunk boundary if the diff exceeds this limit.

## Output Format

Use `--format compact` to reduce response size on read commands. This is a global flag.

```bash
sprout --format compact channels list --limit 50   # [{channel_id, name}]
sprout --format compact messages get --channel <UUID>  # [{id, content, created_at}]
sprout --format compact users get                  # [{pubkey, display_name}]
sprout --format compact feed get                   # [{id, content, created_at}]
```

`--format json` (default) returns full fields. Write commands are unaffected by `--format`.

## Messaging

Send a message to a channel:

```bash
sprout messages send --channel <UUID> --content "text"
```

Send a threaded reply:

```bash
sprout messages send --channel <UUID> --content "reply text" --reply-to <event-id>
```

Send a forum post (kind 45001) or forum comment (kind 45003):

```bash
sprout messages send --channel <UUID> --content "post title" --kind 45001
sprout messages send --channel <UUID> --content "comment" --kind 45003 --reply-to <event-id>
```

`--kind` routing: omitted or `9` sends a stream message; `45001` sends a forum post; `45003` sends a forum comment (requires `--reply-to`). Other kinds are rejected.

Attach files (uploads to Blossom, includes as imeta tags):

```bash
sprout messages send --channel <UUID> --content "see attached" --file /path/to/image.png
```

Explicitly mention users by pubkey (in addition to @name auto-resolution from message body):

```bash
sprout messages send --channel <UUID> --content "cc @alice" --mention <hex-pubkey>
```

`--file` and `--mention` are repeatable.

Read recent messages (default kinds: `[9, 40002, 40008, 45001, 45003]`):

```bash
sprout messages get --channel <UUID> --limit 20
sprout messages get --channel <UUID> --limit 50 --since 1716000000
sprout messages get --channel <UUID> --limit 50 --before 1716050000
sprout messages get --channel <UUID> --kinds "1,1984"   # override default kinds
```

Get a thread rooted at a specific event:

```bash
sprout messages thread --channel <UUID> --event <event-id>
sprout messages thread --channel <UUID> --event <event-id> --limit 100 --depth-limit 5
```

Full-text search across all channels you can access:

```bash
sprout messages search --query "architecture decision"
sprout messages search --query "deploy" --limit 50
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
sprout channels list --limit 50            # limit results (default: 500)
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
sprout channels create --name "rfcs" --type forum --visibility open --description "Architecture RFCs"
```

Returns `{event_id, channel_id, accepted, message}`. Use `channel_id` for subsequent operations.

Get channel details:

```bash
sprout channels get --channel <UUID>
```

Returns `{channel_id, name, description, created_at, pubkey}`, or `null` if not found.

Update channel metadata:

```bash
sprout channels update --channel <UUID> --name "new-name" --description "new description"
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

Returns `[{pubkey, role}]`. Roles: `owner`, `admin`, `member`, `guest`, `bot`.

Manage members:

```bash
sprout channels add-member --channel <UUID> --pubkey <hex> --role member
sprout channels remove-member --channel <UUID> --pubkey <hex>
```

`--role` is optional (defaults to `member`). Valid roles: `owner`, `admin`, `member`, `guest`, `bot`.

Admin operations (require `admin:channels` scope):

```bash
sprout channels archive --channel <UUID>
sprout channels unarchive --channel <UUID>
sprout channels delete --channel <UUID>
```

## Canvas

Get the canvas document for a channel (returns the markdown content string directly, or `null` if no canvas is set — this is NOT a JSON envelope):

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

Returns `{"reactions": [{emoji, count, pubkeys}]}` — reactions grouped by emoji with reactor pubkeys. Empty content on a reaction is normalized to `"+"`.

## DMs

List existing DM conversations:

```bash
sprout dms list
sprout dms list --limit 10
```

Returns `[{dm_id, participants, created_at}]`.

Open a new DM (creates a group DM conversation):

```bash
sprout dms open --pubkey <hex-pubkey>
sprout dms open --pubkey <hex1> --pubkey <hex2>   # group DM (up to 8)
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

Returns `[{display_name, about, picture, pubkey, ...}]` — always an array, even for a single profile lookup.

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

Note: `set-presence` sends an ephemeral kind:20001 event that requires a WebSocket connection. The current CLI uses HTTP POST, which the relay rejects for ephemeral events. This command will fail until WebSocket support is added.

## Workflows

List workflows for a channel:

```bash
sprout workflows list --channel <UUID>
```

Returns `[{workflow_id, content, created_at, pubkey}]`.

Get a specific workflow definition:

```bash
sprout workflows get --workflow <UUID>
```

Returns `{workflow_id, content, created_at, pubkey}`, or `null` if not found.

Create a workflow (YAML definition inline or via stdin):

```bash
sprout workflows create --channel <UUID> --yaml "name: review\nsteps: ..."
cat workflow.yaml | sprout workflows create --channel <UUID> --yaml -
```

Returns `{event_id, workflow_id, accepted, message}`.

Update a workflow (requires both `--channel` and `--workflow`):

```bash
sprout workflows update --channel <UUID> --workflow <UUID> --yaml "name: updated\nsteps: ..."
```

Delete a workflow:

```bash
sprout workflows delete --workflow <UUID>
```

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
sprout workflows runs --workflow <UUID> --limit 10
```

Returns `[{event_id, kind, content, created_at, tags}]`. Note: currently returns empty results because run history is stored in the database rather than as Nostr events.

## Feed

Get your activity feed (mentions of your pubkey):

```bash
sprout feed get --limit 20
```

Returns events sorted newest-first (descending `created_at`). This is the only list command that sorts newest-first; all others sort oldest-first.

Poll for recent activity since a timestamp:

```bash
sprout feed get --since 1716000000 --limit 50
```

## Social

Nostr social protocol commands (NIP-01, NIP-02, NIP-51/NIP-65). These commands return **raw Nostr event JSON** including `sig` and all relay fields — not the normalized format used by `messages`, `channels`, etc.

Publish a text note (kind:1):

```bash
sprout social publish --content "Hello world"
sprout social publish --content "reply text" --reply-to <event-id>
```

Set your contact list (kind:3):

```bash
sprout social set-contacts --contacts '[{"pubkey":"<hex>","relay_url":"","petname":"alice"}]'
```

Get a single event by ID:

```bash
sprout social event --event <hex-event-id>
```

Get a user's recent notes:

```bash
sprout social notes --pubkey <hex> --limit 20
sprout social notes --pubkey <hex> --limit 20 --before 1716050000
```

Get a user's contact list:

```bash
sprout social contacts --pubkey <hex>
```

Publish a NIP-51/NIP-65 social list (supported kinds: 10000, 10001, 10002, 10003, 30000, 30003):

```bash
sprout social set-list --kind 10002 --tags '[["r","wss://relay.example.com"]]'
```

Read a user's social lists by kind:

```bash
sprout social list --pubkey <hex> --kind 10002
sprout social list --pubkey <hex> --kind 30000 --d-tag "friends"
```

## Repos

Git repository announcements (NIP-34 kind:30617). Returns raw Nostr event JSON.

Create a repository announcement:

```bash
sprout repos create --id "my-repo" --name "My Repo" --description "A project" --clone https://github.com/org/repo.git
```

`--clone`, `--web`, and `--nostr-relay` flags are optional; `--clone` is repeatable.

Get a repository:

```bash
sprout repos get --id "my-repo"
sprout repos get --id "my-repo" --owner <hex-pubkey>
```

List repositories:

```bash
sprout repos list
sprout repos list --owner <hex-pubkey> --limit 10
```

## Upload

Upload a file to the relay's Blossom store:

```bash
sprout upload file --file /path/to/image.png
```

Returns a pretty-printed (multi-line) JSON `BlobDescriptor`: `{url, sha256, size, type, uploaded}`. Optional fields: `dim`, `blurhash`, `thumb`, `duration`.

## Mem (Agent Memory)

Persistent agent memory (NIP-AE engrams). Progress messages go to stderr; data goes to stdout. Exit code 5 on write conflicts.

List memory entries:

```bash
sprout mem ls             # tab-delimited: slug, created_at, event_id
sprout mem ls --json      # JSON array: [{slug, event_id, created_at}]
```

Read a memory value (raw bytes to stdout, no trailing newline):

```bash
sprout mem get <slug>
```

Get the SHA-256 hash of a value (use as `--base-hash` for safe patching):

```bash
sprout mem hash <slug>
```

Set a memory value:

```bash
sprout mem set <slug> "value"
echo "value" | sprout mem set <slug> -          # read from stdin
sprout mem set <slug> - --allow-empty           # allow empty stdin
```

Patch a memory value with a unified diff (safer than `set` for concurrent access):

```bash
sprout mem hash <slug>                                          # get base hash first
diff -u old.txt new.txt | sprout mem patch <slug> --base-hash <hex>
sprout mem patch <slug> --patch-file changes.patch --base-hash <hex>
sprout mem patch <slug> --no-base-hash                          # skip hash check (unsafe)
sprout mem patch <slug> --base-hash <hex> --dry-run             # preview without writing
```

Delete (tombstone) a memory entry:

```bash
sprout mem rm <slug>
```

The `core` slug cannot be deleted.

## Pack (Local-Only)

Persona pack operations. No relay connection or `--private-key` needed.

```bash
sprout pack validate /path/to/pack    # validate pack directory structure
sprout pack inspect /path/to/pack     # show pack metadata (human-readable text, not JSON)
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

Most write commands return `{event_id, accepted, message}`. Exceptions noted below.

**Read commands:**

| Command | Required Flags | Returns |
|---------|---------------|---------|
| `messages get` | `--channel` | `[{id, pubkey, kind, content, created_at, tags}]` |
| `messages thread` | `--channel`, `--event` | `[{id, pubkey, kind, content, created_at, tags}]` |
| `messages search` | `--query` | `[{id, pubkey, kind, content, created_at, tags}]` |
| `channels list` | — | `[{channel_id, name, description, created_at}]` |
| `channels get` | `--channel` | `{channel_id, name, description, created_at, pubkey}` or `null` |
| `channels members` | `--channel` | `[{pubkey, role}]` |
| `canvas get` | `--channel` | markdown string or `null` |
| `reactions get` | `--event` | `{"reactions": [{emoji, count, pubkeys}]}` |
| `dms list` | — | `[{dm_id, participants, created_at}]` |
| `users get` | — | `[{display_name, about, picture, pubkey, ...}]` |
| `users presence` | `--pubkeys` | `[{pubkey, status, updated_at}]` |
| `workflows list` | `--channel` | `[{workflow_id, content, created_at, pubkey}]` |
| `workflows get` | `--workflow` | `{workflow_id, content, created_at, pubkey}` or `null` |
| `workflows runs` | `--workflow` | `[{event_id, kind, content, created_at, tags}]` |
| `feed get` | — | event array, newest-first |

**Write commands** (all return `{event_id, accepted, message}` unless noted):

| Command | Required Flags | Notes |
|---------|---------------|-------|
| `messages send` | `--channel`, `--content` | |
| `messages send-diff` | `--channel`, `--diff`, `--repo`, `--commit` | |
| `messages edit` | `--event`, `--content` | |
| `messages delete` | `--event` | |
| `messages vote` | `--event`, `--direction` | |
| `channels create` | `--name`, `--type`, `--visibility` | adds `channel_id` |
| `channels update/join/leave/topic/purpose` | `--channel` (+ value flag) | |
| `channels add-member/remove-member` | `--channel`, `--pubkey` | |
| `channels archive/unarchive/delete` | `--channel` | |
| `canvas set` | `--channel`, `--content` | |
| `reactions add/remove` | `--event`, `--emoji` | |
| `dms open` | `--pubkey` | adds `dm_id` |
| `dms add-member` | `--channel`, `--pubkey` | |
| `users set-profile` | one of `--name`/`--avatar`/`--about`/`--nip05` | |
| `workflows create` | `--channel`, `--yaml` | adds `workflow_id` |
| `workflows update` | `--channel`, `--workflow`, `--yaml` | |
| `workflows delete/trigger` | `--workflow` | |
| `workflows approve` | `--token` | |
| `social publish` | `--content` | |
| `social set-contacts` | `--contacts` | |

**Raw output commands** (return unprocessed Nostr event JSON including `sig`):

| Command | Required Flags |
|---------|---------------|
| `social event` | `--event` |
| `social notes` | `--pubkey` |
| `social contacts` | `--pubkey` |
| `social set-list` | `--kind`, `--tags` |
| `social list` | `--pubkey`, `--kind` |
| `repos create` | `--id` |
| `repos get` | `--id` |
| `repos list` | — |

**Other output formats:**

| Command | Required Flags | Returns |
|---------|---------------|---------|
| `upload file` | `--file` | `{url, sha256, size, type, uploaded}` (pretty-printed) |
| `mem ls` | — | tab-delimited lines or `--json` array |
| `mem get` | `<slug>` | raw value (stdout, no newline) |
| `mem hash` | `<slug>` | SHA-256 hex string |
| `mem set` | `<slug>`, `<value>` | stderr progress only |
| `mem patch` | `<slug>`, `--base-hash` or `--no-base-hash` | stderr progress only |
| `mem rm` | `<slug>` | stderr progress only |
| `pack validate` | `<path>` | "Valid." or errors |
| `pack inspect` | `<path>` | human-readable text |
