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

### Which credentials are used

The CLI uses the first of these that is configured:

1. **An explicit key** (`--private-key` or `BUZZ_PRIVATE_KEY`), with the
   headless NIP-FI settings below when they are set.
2. **The Buzz desktop** (macOS and Linux), through a token it gave this
   process or one saved by `buzz auth desktop pair`. The desktop signs and
   supplies NIP-FI evidence itself; headless NIP-FI settings are ignored, with
   a warning naming them.

With neither, commands fail with an `auth_error`.

### Federated identity (NIP-FI)

A relay that enforces federated identity also needs an identity assertion on
every protected request (`docs/nips/NIP-FI.md`). Give the CLI your NIP-FA
adapter session and it requests one; the adapter mints it
(`docs/nips/NIP-FA.md` §Relay assertion). The CLI caches the assertion for the
process, requests a new one shortly before it expires, and sends it as
`Nostr-Federated-Identity` on HTTP requests and the WebSocket upgrade.

The session is a human sign-in for your own key: the adapter issues
assertions only for the key bound to the signed-in user. It is not an agent
grant, and it cannot authorize an agent key.

```bash
export BUZZ_PRIVATE_KEY="nsec1..."
export BUZZ_FEDERATED_IDENTITY_ADAPTER="https://adapter.example"
export BUZZ_FEDERATED_IDENTITY_TOKEN="..."   # your adapter session token; env only, never a flag
buzz channels list
```

The token goes only to the adapter, never through a redirect, and never into
output. Adapter outages (network errors, 429, 5xx) are retried a bounded number
of times and then fail as a retryable `adapter_error` before anything reaches
the relay; an adapter refusal is an `auth_error` and is not retried. On relays without NIP-FI, leave both unset: nothing changes. An
enforcing relay's `401` without them is an `auth_error` naming both settings.

## Usage

All output is JSON on stdout. Errors are JSON on stderr. Exit codes: 0=ok, 1=user error, 2=network, 3=auth, 4=other, 5=write conflict.

```bash
# Set relay URL (defaults to http://localhost:3000)
export BUZZ_RELAY_URL="https://relay.example.com"

# Messages
buzz messages send --channel <uuid> --content "Hello"
buzz messages send --channel <uuid> --content "Reply" --reply-to <event-id> --broadcast
buzz messages send --channel <uuid> --content - < message.md   # read body from stdin
buzz messages send --channel <uuid> --content "Decision: ship" --from-thread <root-event-id>   # desktop "Send to channel"
buzz messages get --channel <uuid> --limit 20
buzz messages thread --channel <uuid> --event <event-id>
buzz messages thread --link 'buzz://message?channel=<uuid>&id=<event-id>&thread=<root-id>'
buzz messages search --query "architecture"
buzz messages search --author <pubkey|npub|name> --since <unix-ts>
buzz messages edit --event <event-id> --content "Updated text"
buzz messages delete --event <event-id>
buzz messages report --event <event-id> --reason spam --note "why"   # to moderators only
buzz messages report --pubkey <pubkey> --reason impersonation

# Diffs
buzz messages send-diff --channel <uuid> --diff - --repo https://github.com/org/repo --commit abc123 < diff.patch

# Channels
buzz channels list
buzz channels unread                      # unread counts from your synced read state (read-only)
buzz channels create --name "my-channel" --type stream --visibility open
buzz channels join --channel <uuid>
buzz channels topic --channel <uuid> --topic "New topic"

# Reactions
buzz reactions add --event <event-id> --emoji "👍"
buzz reactions get --event <event-id>

# Huddles (read-only)
buzz huddles list --channel <uuid> --active   # huddles with state and start/end times
buzz huddles participants --channel <uuid> --huddle <huddle-id>

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
buzz users set-status --text "lunch" --emoji "🥪" --expires 1h  # clears itself; also a unix time
buzz users set-status --clear                 # remove your status
buzz users status                       # your visible status: [{pubkey, text, emoji, expires_at, updated_at}]
buzz users status --pubkey <hex|npub|name>  # repeatable, max 100; expired or cleared users are omitted

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

# Sidebar sections (your own sidebar, synced with Buzz desktop)
buzz sidebar sections create --name "Engineering" --icon "🛠"
buzz sidebar sections move --section "Engineering" --channel <uuid> --channel <uuid>
buzz sidebar sections list

# Sidebar sort modes (alpha or recent per group)
buzz sidebar sort set --group dms --mode recent
buzz sidebar sort set --group "Engineering" --mode recent
buzz sidebar sort reset --group section:<section-id>
buzz sidebar sort list

# Sidebar stars, mutes, and project pins
buzz sidebar stars add --channel <uuid> --channel <uuid>
buzz sidebar mutes remove --channel <uuid>
buzz sidebar projects add --project 30621:<owner-pubkey>:<d-tag>
buzz sidebar stars list

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

`sidebar sections` edits the signing identity's own sidebar. Each command is
one read and one publish, so a multi-channel `move` lands together. `--section`
takes a section id or a unique section name. A repeated command prints
`"message": "unchanged"` and publishes nothing. Concurrent edits from another
CLI run or another client can overwrite each other, and the last publish wins.

`sidebar stars`, `sidebar mutes`, and `sidebar projects` edit the other synced
sidebar lists the same way: one head read and one publish per command, and
`"message": "unchanged"` when nothing changes. `remove` keeps a `false` entry
(as desktop does) so the change syncs; removing an id that is not set changes
nothing. Stars and mutes keep the newest 500 entries; a command that would
evict its own ids fails. Project addresses are `30621:<owner-pubkey>:<d-tag>`.
Pass the same ids in any case; channel UUIDs and project owners are stored in
lowercase. If the relay reports a newer update (exit 5), run the command again.

`sidebar sort` sets how one sidebar group orders its channels: `alpha` (name,
the default) or `recent` (last message, newest first). `--group` takes
`starred`, `channels`, `forums`, `dms`, `section:<id>` (an exact live id), or a
section name (or bare id). A fixed group name wins over a section with the same name. `sort list`
prints `{"default": "alpha", "groups": {<key>: <mode>}}` with only the groups
that have a mode set. `sort reset` writes a `null` register that returns the
group to the default; resetting a group with no register changes nothing. A
write is one sort-head read and one publish; a section name also reads your
sections. Desktop keeps a deleted section's `section:<id>` mode, and so does the
CLI: it still shows in `sort list`, `sort set` refuses it, and `sort reset
--group section:<id>` clears it. The sidebar keeps at most 104 sort modes
(stale keys count), and a command that would pass that fails.

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
| | `report` | Report a message or user to community moderators (NIP-56); reasons: `illegal`, `nudity`, `malware`, `spam`, `impersonation`, `profanity`, `other` |
| | `get` | List messages in a channel |
| | `thread` | Get a message thread |
| | `search` | Full-text search, filterable by author |
| | `vote` | Vote on a forum post |
| `channels` | `list` | List channels |
| | `get` | Get channel details |
| | `unread` | Unread counts per channel (read-only) |
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
| `sidebar` | `sections list` | List your sections and their channels |
| | `sections create` | Create a section at the end of your sidebar |
| | `sections update` | Rename a section or change its icon |
| | `sections delete` | Delete a section |
| | `sections reorder` | Put sections in order |
| | `sections move` | Move one or more channels into a section |
| | `sections unassign` | Return channels to the default channel list |
| | `sort list` | List the groups with a sort mode |
| | `sort set` | Sort a group by `alpha` or `recent` |
| | `sort reset` | Return a group to the default sort |
| | `stars list` / `add` / `remove` | List, star, or unstar channels |
| | `mutes list` / `add` / `remove` | List, mute, or unmute channels |
| | `projects list` / `add` / `remove` | List, pin, or unpin sidebar projects |
| `reactions` | `add` | React to a message |
| | `remove` | Remove a reaction |
| | `get` | List reactions |
| `huddles` | `list` | List a channel's huddles (read-only) |
| | `participants` | Show who is in a huddle (read-only) |
| `gifs` | `search` | Search or browse trending GIFs (requires relay buzz-gif support) |
| | `share` | Report a selected GIF to the provider's Recents |
| `dms` | `list` | List DM conversations |
| | `open` | Open a DM (1–8 pubkeys) |
| | `add-member` | Add member to DM group |
| `users` | `get` | Get user profile(s) |
| | `set-profile` | Update your profile |
| | `presence` | Get presence status |
| | `set-presence` | Set presence status |
| | `set-status` | Set or clear your NIP-38 profile status; `--expires` takes a unix time or `30m`/`2h`/`1d`/`1w` |
| | `status` | Read NIP-38 statuses (default: yours); expired or cleared are omitted |
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

## Huddles

`huddles list` and `huddles participants` rebuild huddle state from the relay-signed lifecycle
events (kinds 48100–48103) with the same rules as the desktop app. They never join, start, or end
a huddle. Each scans at most 5000 newest events of the channel. If older events may be missing,
`complete` is `false` and `truncated` is `true`. A huddle with no end event is `active`, then
`stale` once its start is over an hour old; desktop shows `stale` as ended. The relay's liveness
snapshots are not served over HTTP, so `liveness_checked` is always `false`.
Rosters come from the relay's join and leave events only. Desktop's roster view also lists the
creator from the start event; the CLI lists the creator once the relay records their join.

## Architecture

```
buzz <group> <subcommand> [flags]
    │
    ├─ main.rs ──▶ commands/*.rs ──▶ client.rs ──▶ Buzz Relay REST API
    │  (clap)       (handlers)       (reqwest)
    │
    ├─ credentials.rs        (identity, signer, NIP-FI evidence; every request
    │                         is authorized through client.rs `authorize`)
    ├─ federated_identity.rs (NIP-FA adapter session → NIP-FI assertion)
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
