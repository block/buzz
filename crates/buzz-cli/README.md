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
# Names resolve with one batched lookup of catalog entries (kind:30180), then
# the legacy palette only for names still unresolved. If a lookup fails, the
# message is still sent, without emoji tags, and a warning goes to stderr.

# Catalog emoji (one event per emoji)
buzz emoji search party                 # prefix search, min 2 chars; [{shortcode,url,author,event_id,created_at}]
buzz emoji search party --limit 50      # default 20, max 50
buzz emoji resolve wave party_parrot    # exact lookup; {"found": [...], "missing": [...]}
# If two authors publish the same shortcode, the newest event is returned and
# the other authors are listed in `other_authors`.
# `buzz emoji list` shows your own set and the legacy union only; use
# `emoji search` to find catalog emoji.
# Do NOT run `buzz emoji import` with a large list (for example a Slack
# export). It builds one very large event that the relay rejects, and stdin
# input is cut at 10 MB.

# Bulk-migrate a Slack catalog (one kind:30180 event per emoji): see
# "Migrating a Slack emoji catalog" below.
buzz emoji migrate plan --slack-json emoji.json --dry-run

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

## Migrating a Slack emoji catalog

`buzz emoji migrate {plan,run,verify,rollback}` loads a large Slack catalog
(tens of thousands of emoji) as one kind:30180 event per emoji. Use it instead
of `emoji import`, which builds one event and cannot carry a catalog this size.
Every command takes `--work-dir` (default `./emoji-migration`), which holds:

| File | Written by | Meaning |
|------|------------|---------|
| `plan.json` | `plan` | the names and image sources `run` will migrate |
| `rejects.jsonl` | `plan` | one line per skipped name with a `reason` |
| `dry-run-rejects.jsonl` | `plan --dry-run` | images that failed to fetch or sanitize |
| `journal.jsonl` | `run`, `verify`, `rollback` | append-only record of every state change; the source of truth |
| `snapshot.json` | `run` | state counts for the operator (replay never reads it) |
| `run-rejects.jsonl` | `run` | items the run gave up on, with a reason |
| `blobs/` | `run` | sanitized bytes waiting for upload; safe to delete when no run is active |

**Input.** `--slack-json emoji.json` takes the Slack `emoji.list` shape,
`{"emoji": {"name": "<url>", "alias": "alias:name"}}`. `--images-dir dir`
takes a directory of images (names come from file names), optionally with
`--name-map map.json` (`{"name": "file.png", "other": "alias:name"}`; file names
only, no paths). Names follow the relay's shortcode rules (lowercase, `a-z0-9_-`,
64 bytes). A name that breaks them, such as `+1`, is rejected with `invalid_name`;
the tool never renames. An alias becomes a second entry that shares the target's
image. Alias chains longer than 5 hops, cycles and dangling aliases go to
`rejects.jsonl`. An alias to a standard Unicode emoji is reported as
`alias_dangling` unless `--unicode-names list.txt` (one shortcode per line)
lets the tool report it as `alias_to_unicode`.

**Safe by default.** `plan` never writes to Buzz. It reads the relay only to
find names that already exist (`--on-conflict skip|overwrite`, default `skip`).
`--no-conflict-check` skips even that read. `run` needs the key of a community
member; use one dedicated migration key, because the community then owns the
emoji under that key and members cannot delete them. Sanitizing is a structural
strip of metadata chunks (PNG text, EXIF, ICC; GIF comments; JPEG APPn; WebP
EXIF/XMP/ICC) and never re-encodes pixels. The relay's own validator checks every
result before upload. Images are de-duplicated by the SHA-256 of the sanitized
bytes.

**Limits and pacing.** `--uploads-per-minute` (default 30, at most 2 uploads at
once), `--events-per-minute` (default 240) and `--queries-per-minute` (default
120) pace each action evenly, with no burst, and a relay `retry in Ns` hint
pauses the action. `--concurrency` (default 8) bounds items in flight,
`--max-bytes` (default 1 MiB) caps each download, and `--max-attempts` (default
3) bounds retries per item and stage; an item that runs out becomes `rejected`.
A 401 or 403 stops the whole run instead of rejecting items. The tool works at
the default rates. Ask the relay operator to raise
`BUZZ_MEDIA_UPLOADS_PER_MINUTE` and `BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN` to
shorten a very large run.

**Resuming.** `run` replays `journal.jsonl` and continues after any stop,
including a crash or a power loss. It writes a record, syncs it to disk, and only
then takes the next action. It refuses a journal that belongs to a different
`plan.json`; `--force-new-plan` moves the old journal aside (it is never
deleted). Keep the journal until `verify` passes with zero failures.

### Runbook

1. **Dry run.** Fetches and sanitizes every image and writes `plan.json`.
   Nothing is written to Buzz.

   ```bash
   buzz emoji migrate plan --slack-json emoji.json --dry-run --work-dir ./mig
   ```

   Read the JSON report: `rejected_by_reason`, `duplicate_ratio`,
   `bytes_to_upload` and `estimate` (seconds under the rate flags). Read
   `rejects.jsonl` and `dry-run-rejects.jsonl`. For a faster look at a huge export
   use `--dry-run --sample 2000`, which extrapolates (`estimate_basis` says so).
2. **Pilot of 1,000 on staging.** Point `BUZZ_RELAY_URL` and `BUZZ_PRIVATE_KEY`
   at staging and the dedicated migration key. Cut a 1,000-name Slack file
   (for example `jq '{emoji: (.emoji | to_entries | .[:1000] | from_entries)}'
   emoji.json > pilot.json`), then:

   ```bash
   buzz emoji migrate plan --slack-json pilot.json --work-dir ./pilot
   buzz emoji migrate run --work-dir ./pilot
   buzz emoji migrate verify --work-dir ./pilot --probe-blobs 25
   ```

   Spot-check with `buzz emoji search <prefix>` and in the desktop picker.
3. **Rollback test.** Delete the pilot and confirm it is gone:

   ```bash
   buzz emoji migrate rollback --work-dir ./pilot --dry-run   # counts only
   buzz emoji migrate rollback --work-dir ./pilot --yes
   buzz emoji resolve <a pilot name>                           # now "missing"
   ```

   Blobs stay on the relay. A later `run` on the same work dir publishes the
   entries again without uploading.
4. **Full run.** Use a fresh work dir for the real plan, then run. Re-run the same
   command after any interruption.

   ```bash
   buzz emoji migrate plan --slack-json emoji.json --work-dir ./mig
   buzz emoji migrate run --work-dir ./mig
   ```

   `run` exits non-zero if any item was rejected; the reasons are in
   `run-rejects.jsonl`.
5. **Verify.**

   ```bash
   buzz emoji migrate verify --work-dir ./mig --probe-blobs 50
   ```

   `verify` checks every published name with `#d` lookups (50 names per query),
   compares each URL with the journal, probes `--probe-blobs` random blobs with a
   `HEAD` request (default 0, because a relay's `HEAD` behavior is not confirmed),
   and runs `--prefix-searches` (default 20) random prefix searches. A search whose
   result list is full without the name is counted as inconclusive, not failed. It
   exits non-zero on any failure and lists each one. Only after it passes with zero
   failures may you archive the work dir.

