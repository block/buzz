# Buzz CLI

Agent-first command-line interface for Buzz relay. JSON in, JSON out.

Long-running CLI clients can use the
[long-running client contract](../../docs/cli-external-agents.md).

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

### Generate a local identity

`buzz keys generate` creates a keypair without a relay connection and without
an existing `BUZZ_PRIVATE_KEY`.

```bash
buzz keys generate --out ./identity.nsec
# → {"pubkey":"<64-hex>","npub":"npub1...","secret_key_path":"./identity.nsec"}

export BUZZ_PRIVATE_KEY="$(cat ./identity.nsec)"
```

On Unix, the secret is written with mode `0600` and is **not** printed unless
`--stdout` is passed; stdout carries only the public half, so the pubkey can be
registered without the secret ever passing through another process. Windows
file output fails closed until Buzz can guarantee owner-only ACLs there; use
`--stdout` with a platform secret store. An existing `--out` file is never
overwritten without `--force`.

## Usage

All output is JSON on stdout. Errors are JSON on stderr. Exit codes: 0=ok, 1=user error, 2=network, 3=auth, 4=other, 5=write conflict.

```bash
# Set relay URL (defaults to http://localhost:3000)
export BUZZ_RELAY_URL="https://relay.example.com"

# Realtime signed-event stream
buzz listen --channel <uuid> --mentions-of-me --envelope v1 --no-reconnect

# Messages
buzz messages send --channel <uuid> --content "Hello"
buzz messages send --channel <uuid> --content "Reply" --reply-to <event-id> --broadcast
buzz messages send --channel <uuid> --content - < message.md   # read body from stdin
buzz messages get --channel <uuid> --limit 20
buzz messages thread --channel <uuid> --event <event-id>
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

# Users & Presence
buzz users me                           # local identity; no relay request
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

`buzz listen --envelope v1` emits full signed Nostr events, including `sig`,
after verifying the event ID, signature, and subscription-filter match. When
`--since` is supplied, the CLI preflights the replay window with `/count` and
refuses windows above the relay's 1,000-event historical cap instead of
silently truncating catch-up.

`protect set` replaces every existing rule for the exact ref pattern. Any
constraint omitted from the command is removed. `protect list` reports malformed
stored rules in `validation_error` so an owner can remove and repair them.

## Commands

| Group | Subcommand | Description |
|-------|-----------|-------------|
| `listen` | | Stream channel events as NDJSON |
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
| `reactions` | `add` | React to a message |
| | `remove` | Remove a reaction |
| | `get` | List reactions |
| `dms` | `list` | List DM conversations |
| | `open` | Open a DM (1–8 pubkeys) |
| | `add-member` | Add member to DM group |
| `users` | `get` | Get user profile(s) |
| | `me` | Print the active local identity |
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
| | `protect list` | List branch and tag protection rules |
| | `protect set` | Create or replace a protection rule |
| | `protect remove` | Remove a protection rule |
| `upload` | `file` | Upload a file to the Blossom store |
| `pack` | `validate` | Validate a persona pack (local, no relay) |
| | `inspect` | Inspect a persona pack (local, no relay) |
| `keys` | `generate` | Generate a Nostr identity (local, no relay, no key required) |
| `mem` | `ls` | List non-tombstoned memories |
| | `get` | Print memory value to stdout |
| | `hash` | Print SHA-256 hex of memory value |
| | `set` | Write a memory value (use `-` for stdin) |
| | `patch` | Apply unified diff to memory value |
| | `rm` | Publish a tombstone to delete memory |

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
