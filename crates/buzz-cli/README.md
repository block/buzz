# Buzz CLI

Agent-first command-line interface for Buzz relay. JSON in, JSON out.

## Install

```bash
cargo install --path crates/buzz-cli
```

## Authentication

| Env Var | Mode | Use Case |
|---------|------|----------|
| `BUZZ_PRIVATE_KEY` | NIP-98 Schnorr signature | Agents with a keypair |

```bash
# Private key identity (NIP-98 signed requests)
export BUZZ_PRIVATE_KEY="nsec1..."
buzz channels list
```

## Usage

All output is JSON on stdout. Errors are JSON on stderr. Exit codes: 0=ok, 1=user error, 2=network, 3=auth, 4=other, 5=write conflict.

```bash
# Set relay URL (defaults to http://localhost:3000)
export BUZZ_RELAY_URL="https://relay.example.com"

# Messages
buzz messages send --channel <uuid> --content "Hello"
buzz messages send --channel <uuid> --content "Reply" --reply-to <event-id> --broadcast
buzz messages send --channel <uuid> --content - < message.md   # read body from stdin
buzz messages get --channel <uuid> --limit 20
buzz messages thread --channel <uuid> --event <event-id>
buzz messages thread --link 'buzz://message?channel=<uuid>&id=<event-id>&thread=<root-id>'
buzz messages search --query "architecture"
buzz messages search --author <pubkey|npub|name> --since <unix-ts>
buzz messages edit --event <event-id> --content "Updated text"
buzz messages delete --event <event-id>

# Diffs
buzz messages send-diff --channel <uuid> --diff - --repo https://github.com/org/repo --commit abc123 < diff.patch

# Channels
buzz channels list
buzz channels create --name "my-channel" --type stream --visibility open
buzz channels join --channel <uuid>
buzz channels topic --channel <uuid> --topic "New topic"

# Reactions
buzz reactions add --event <event-id> --emoji "👍"
buzz reactions get --event <event-id>

# GIFs (requires relay to advertise buzz-gif / KLIPY)
buzz gifs search                              # trending GIFs
buzz gifs search --query "celebration"        # search GIFs
buzz gifs share --slug <slug>                 # report selection to provider Recents
# Paste the `cdn_url` from a search result directly into messages send --content

# Custom emoji in messages
# buzz messages send scans outgoing content for :shortcode: patterns and
# automatically attaches NIP-30 ["emoji", shortcode, url] tags from the
# workspace palette — identical to the desktop composer behavior.

# Users & Presence
buzz users get                          # your own profile
buzz users get --pubkey <hex>           # single user
buzz users get --pubkey <hex> --pubkey <hex>  # batch (max 200)
buzz users get --name Honey --owner me  # exact-name lookup in your managed agents
buzz users set-presence --status online
buzz users set-status --text "heads down on the CLI" --emoji "🚀"
buzz users set-status --clear                 # remove your status

# DMs
buzz dms open --pubkey <hex>
buzz dms list

# Workflows
buzz workflows list --channel <uuid>
buzz workflows trigger --workflow <uuid>
buzz workflows approve --token <uuid>
buzz workflows approve --token <uuid> --approved false --note "needs revision"

# Forum
buzz messages vote --event <event-id> --direction up

# Canvas
buzz canvas get --channel <uuid>
buzz canvas set --channel <uuid> --content "# Welcome"

# Channel artifacts (NIP-AR): typed, titled, editable records in a channel
buzz artifacts create --channel <uuid> --type buzz.task --title "Fix timeout" --content '{"version":1}'
buzz artifacts list --channel <uuid> --type buzz.task
buzz artifacts get --channel <uuid> --id <artifact-uuid>
buzz artifacts update --channel <uuid> --id <artifact-uuid> --content-file task.json
buzz artifacts move --channel <uuid> --id <artifact-uuid> --to <other-channel-uuid>  # shows the audience; add --yes to confirm
buzz artifacts delete --channel <uuid> --id <artifact-uuid>
buzz artifacts create --channel <uuid> --type buzz.config --title Theme --ensure theme  # idempotent create-or-reconcile

# Agent Memory (NIP-AE)
buzz mem ls
buzz mem get <slug>
buzz mem set <slug> "my-value"
buzz mem patch <slug> --base-hash <hex> < diff.patch  # or --no-base-hash
buzz mem rm <slug>

# Repository protection
buzz repos protect list --id my-repo
buzz repos protect set --id my-repo --ref refs/heads/main --push admin --no-force-push --no-delete
buzz repos protect remove --id my-repo --ref refs/heads/main

# Pipe to jq
buzz channels list | jq '.[].name'
```

`protect set` replaces every existing rule for the exact ref pattern. Any
constraint omitted from the command is removed. `protect list` reports malformed
stored rules in `validation_error` so an owner can remove and repair them.

## Commands

`buzz --help` prints this whole surface as an indented tree — every group with
its subcommands and their descriptions — so there is no need to run `--help`
once per group to find a command. `buzz -h` keeps the short group-level
summary, and `buzz <group> <subcommand> --help` has the flags and examples.
The table below mirrors that tree for readers who are not at a terminal.

| Group | Subcommand | Description |
|-------|-----------|-------------|
| `messages` | `send` | Send a message to a channel |
| | `send-diff` | Send a code diff with metadata |
| | `edit` | Edit a message you sent |
| | `delete` | Delete a message |
| | `get` | List messages in a channel |
| | `thread` | Get a message thread |
| | `search` | Full-text search, filterable by author |
| | `vote` | Vote on a forum post |
| `channels` | `list` | List channels |
| | `get` | Get channel details |
| | `create` | Create a channel |
| | `update` | Update channel name/description |
| | `topic` | Set channel topic |
| | `purpose` | Set channel purpose |
| | `join` | Join a channel |
| | `leave` | Leave a channel |
| | `archive` | Archive a channel |
| | `unarchive` | Unarchive a channel |
| | `delete` | Delete a channel |
| | `members` | List channel members |
| | `add-member` | Add a member |
| | `remove-member` | Remove a member |
| `canvas` | `get` | Get channel canvas |
| | `set` | Set channel canvas |
| `artifacts` | `list` | List the current revision of each live artifact in a channel |
| | `get` | Get one live artifact |
| | `create` | Create an artifact (`--ensure KEY` for an idempotent derived id) |
| | `update` | Edit title/content; keeps id, type, channel and other tags |
| | `move` | Move an artifact into another channel (`--yes` to confirm) |
| | `delete` | Soft-delete an artifact |
| | `restore` | Restore a deleted artifact from a new snapshot |
| `reactions` | `add` | React to a message |
| | `remove` | Remove a reaction |
| | `get` | List reactions |
| `gifs` | `search` | Search or browse trending GIFs (requires relay buzz-gif support) |
| | `share` | Report a selected GIF to the provider's Recents |
| `dms` | `list` | List DM conversations |
| | `open` | Open a DM (1–8 pubkeys) |
| | `add-member` | Add member to DM group |
| `users` | `get` | Get user profile(s) |
| | `set-profile` | Update your profile |
| | `presence` | Get presence status |
| | `set-presence` | Set presence status |
| | `set-status` | Set or clear your NIP-38 profile status |
| `workflows` | `list` | List workflows |
| | `get` | Get workflow definition |
| | `create` | Create a workflow |
| | `update` | Update a workflow |
| | `delete` | Delete a workflow |
| | `trigger` | Trigger a workflow |
| | `runs` | Get workflow run history |
| | `approve` | Approve/deny a workflow step |
| `feed` | `get` | Get your activity feed |
| `social` | `publish` | Publish a NIP-01 note |
| | `set-contacts` | Set NIP-02 contact list |
| | `event` | Get a Nostr event |
| | `notes` | Get notes for a user |
| | `contacts` | Get NIP-02 contact list |
| `repos` | `create` | Announce a git repository (NIP-34) |
| | `get` | Get a repository announcement |
| | `list` | List repository announcements |
| | `default-branch get/set` | Read or select an existing default branch (requires relay support) |
| | `protect list` | List branch and tag protection rules |
| | `protect set` | Create or replace a protection rule |
| | `protect remove` | Remove a protection rule |
| `upload` | `file` | Upload a file to the Blossom store |
| `pack` | `validate` | Validate a persona pack (local, no relay) |
| | `inspect` | Inspect a persona pack (local, no relay) |
| `mem` | `ls` | List non-tombstoned memories |
| | `get` | Print memory value to stdout |
| | `hash` | Print SHA-256 hex of memory value |
| | `set` | Write a memory value (use `-` for stdin) |
| | `patch` | Apply unified diff to memory value |
| | `rm` | Publish a tombstone to delete memory |

### Channel artifacts (NIP-AR)

`buzz artifacts` reads and writes kind 45010 revisions. An artifact has a
stable UUID (`d`), one home channel (`h`), an immutable namespaced `type`
(lowercase dot-separated, at least two parts, for example `buzz.task`), a
`title` (at most 512 bytes, visible to every reader of the channel), and
opaque `content` text (at most 64 KiB). The CLI defines no content schema.

- `list` returns the current revision of each live artifact, at most
  `--limit` (default 100, maximum 1000). Deleted artifacts are omitted. Narrow
  with `--type` when a channel holds more.
- `get` returns one live artifact as a one-element array. An unknown,
  deleted, moved or unreadable artifact is not-found (exit 1).
- `update` publishes a complete snapshot against the current revision. It
  keeps `d`, `h`, `type`, the conversation anchor and every client-defined
  tag, and changes only the fields you pass.
- `move` publishes the current snapshot into another channel under the same
  `d` and drops the conversation anchor, which belongs to the source. Everyone
  who reads the destination can read it, so without `--yes` the command writes
  nothing and prints the destination name, visibility and what would be shared.
- `delete` is a soft delete; the id stays reserved. `restore` revives it from a
  complete new snapshot (`--title` is required).
- `create --ensure KEY` derives `d` from the channel, type and key
  (`UUIDv8(SHA-256("nip-ar-artifact\0" + h + "\0" + type + "\0" + key))`), so
  independent clients agree on it. If the artifact already exists the command
  restores it when deleted, does nothing when the title and content already
  match, and otherwise updates it, trying at most 3 times.

**Concurrent edits.** Every write after create names the current revision in
`prev`, and the relay accepts only one of two competing writes. The loser gets
exit code 5 (`conflict`). Run `buzz artifacts get`, reapply your change to the
current revision, and retry. The CLI never retries a conflict with only `prev`
replaced, because that would overwrite the competing edit. Only
`create --ensure` retries on its own.

Reads are bounded `POST /query` artifact views (`current` and `head`); the CLI
does not use sync subscriptions or incremental reads.

## Architecture

```
buzz <group> <subcommand> [flags]
    │
    ├─ main.rs ──▶ commands/*.rs ──▶ client.rs ──▶ Buzz Relay REST API
    │  (clap)       (handlers)       (reqwest)
    │
    ├─ validate.rs   (UUID, hex, content size, percent-encode)
    └─ error.rs      (CliError → JSON stderr + exit code)

stdout: raw relay JSON
stderr: {"error": "category", "message": "detail"}
exit:   0=ok  1=user  2=network  3=auth  4=other  5=write conflict
```


### Default branch

After deploying relay support, select an existing published branch without
renaming or deleting any branch:

```bash
buzz repos default-branch get --owner <owner-hex> --id my-repo
buzz repos default-branch set --owner <owner-hex> --id my-repo --branch main
# For an explicitly reviewed version, use the manifest digest returned by get:
buzz repos default-branch set --owner <owner-hex> --id my-repo --branch main \
  --expected-manifest <manifest-digest>
```

`--owner` defaults to the signing identity, not an agent's attested human owner.
`set` without `--expected-manifest` reads the current version first. Success
returns `branch`, `head`, `manifest` and `changed`; `get` omits `changed`.
A stale version returns conflict (exit 5). Ambiguous write failures return
`delivery_unknown` with `retryable:false` and the original digest: **read before
retrying**, and do not blindly re-run against a newly fetched version.

The signer must be a current channel member and a repository manager, directly
or through an unrestricted, valid NIP-OA owner attestation; permission to push is
not permission to change the default. See the
[protocol and authorization contract](../../docs/git-on-object-storage.md#default-branch-management).
