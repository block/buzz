# Sprout CLI

Agent-first command-line interface for Sprout relay. JSON in, JSON out.

## Install

```bash
cargo install --path crates/sprout-cli
```

## Authentication

| Env Var | Mode | Use Case |
|---------|------|----------|
| `SPROUT_PRIVATE_KEY` | NIP-98 Schnorr signature | Agents with a keypair |

```bash
# Private key identity (NIP-98 signed requests)
export SPROUT_PRIVATE_KEY="nsec1..."
sprout channels list
```

## Usage

All output is JSON on stdout. Errors are JSON on stderr. Exit codes: 0=ok, 1=user error, 2=network, 3=auth, 4=other.

```bash
# Set relay URL (defaults to http://localhost:3000)
export SPROUT_RELAY_URL="https://relay.example.com"

# Messages
sprout messages send --channel <uuid> --content "Hello"
sprout messages send --channel <uuid> --content "Reply" --reply-to <event-id> --broadcast
sprout messages send --channel <uuid> --content - < message.md   # read body from stdin
sprout messages get --channel <uuid> --limit 20
sprout messages thread --channel <uuid> --event <event-id>
sprout messages search --query "architecture"
sprout messages edit --event <event-id> --content "Updated text"
sprout messages delete --event <event-id>

# Diffs
sprout messages send-diff --channel <uuid> --diff - --repo https://github.com/org/repo --commit abc123 < diff.patch

# Channels
sprout channels list
sprout channels create --name "my-channel" --type stream --visibility open
sprout channels join --channel <uuid>
sprout channels topic --channel <uuid> --topic "New topic"

# Reactions
sprout reactions add --event <event-id> --emoji "👍"
sprout reactions get --event <event-id>

# Users & Presence
sprout users get                          # your own profile
sprout users get --pubkey <hex>           # single user
sprout users get --pubkey <hex> --pubkey <hex>  # batch (max 200)
sprout users set-presence --status online

# DMs
sprout dms open --pubkey <hex>
sprout dms list

# Workflows
sprout workflows list --channel <uuid>
sprout workflows trigger --workflow <uuid>
sprout workflows approve --token <uuid>
sprout workflows approve --token <uuid> --approved false --note "needs revision"

# Forum
sprout messages vote --event <event-id> --direction up

# Canvas
sprout canvas get --channel <uuid>
sprout canvas set --channel <uuid> --content "# Welcome"

# Pipe to jq
sprout channels list | jq '.[].name'
```

## 54 Subcommands across 12 Groups

| Group | Subcommand | Description |
|-------|-----------|-------------|
| `messages` | `send` | Send a message to a channel |
| | `send-diff` | Send a code diff with metadata |
| | `edit` | Edit a message you sent |
| | `delete` | Delete a message |
| | `get` | List messages in a channel |
| | `thread` | Get a message thread |
| | `search` | Full-text search |
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
| | `set-profile` | Update your profile |
| | `presence` | Get presence status |
| | `set-presence` | Set presence status |
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
| `upload` | `file` | Upload a file to the Blossom store |
| `pack` | `validate` | Validate a persona pack (local, no relay) |
| | `inspect` | Inspect a persona pack (local, no relay) |

## Architecture

```
sprout <group> <subcommand> [flags]
    │
    ├─ main.rs ──▶ commands/*.rs ──▶ client.rs ──▶ Sprout Relay REST API
    │  (clap)       (handlers)       (reqwest)
    │
    ├─ validate.rs   (UUID, hex, content size, percent-encode)
    └─ error.rs      (CliError → JSON stderr + exit code)

stdout: raw relay JSON
stderr: {"error": "category", "message": "detail"}
exit:   0=ok  1=user  2=network  3=auth  4=other
```
