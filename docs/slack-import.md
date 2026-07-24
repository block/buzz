# Slack Import

`buzz import slack` migrates a Slack workspace export into a Buzz community.
Buzz was built to reduce dependency on Slack; this tool is the on-ramp — it
carries a team's conversational history (channels, messages, threads,
reactions) onto a relay you own, so agents and people can search it as one
record from day one.

```bash
buzz import slack --export-dir ./my-workspace-export            # bot mode
buzz import slack --export-dir ./export --mapping keys.json     # mapping mode
buzz import slack --export-dir ./export --dry-run               # plan only
```

## What gets imported

| Slack | Buzz | Notes |
|-------|------|-------|
| Channels (`channels.json`) | kind `9007` create + `9002` topic/purpose | UUID generated per channel, recorded in the state file |
| Messages (per-day JSON) | kind `9` stream message, `h`-tagged | `created_at` backdated to the original Slack `ts` |
| Threads (`thread_ts`) | NIP-10 `e` reply tags | Slack threads are flat; every reply is a direct reply to the root |
| Reactions | kind `7` | Common shortcodes mapped to Unicode, otherwise `:shortcode:` |
| Users (`users.json`) | kind `0` profiles | Mapping mode only — signed by each user's key |
| Files | Links appended to message content | Blobs are **not** downloaded/re-hosted (see Limitations) |
| Custom emoji | — | Use `scripts/grab-emoji.sh` (separate tool, needs a Slack API token) |

Every imported event carries provenance tags:

- `["import", "slack"]` — marks the event as imported
- `["import_author", "<slack user id>", "<display name>"]` — original author
- `["import_ts", "<slack ts>"]` — original microsecond-precision timestamp
  (Nostr `created_at` is seconds, so this preserves sub-second ordering data)

## Identity modes

### Bot mode (default)

Everything is signed by the CLI identity (`BUZZ_PRIVATE_KEY`). Message
content is prefixed with the original author's display name
(`**Alice**: …`) so history stays readable; machine-readable attribution
lives in the `import_author` tag.

- Zero key custody — no keys are generated or distributed.
- History is attributed to the importer identity, not to individual people.

### Mapping mode (`--mapping keys.json`)

A JSON file maps Slack user IDs to Nostr private keys:

```json
{
  "U01ABCDEF": { "private_key": "nsec1..." },
  "U02GHIJKL": { "private_key": "<64-char hex>" }
}
```

Messages and reactions from mapped users are signed with *their* keys, so
imported history is natively attributable — six months from now, "my
messages" really are that pubkey's messages. Unmapped users (departed
members, bots) fall back to bot-mode signing with the author-name prefix.

Requirements and behavior:

- Every event is signed locally with the mapped user's key and submitted
  over the single CLI connection. The relay accepts the author/submitter
  mismatch because the events carry `import` provenance tags and the CLI
  identity is a community owner/admin (see the exemption below) — the
  event's own Schnorr signature proves authorship. Mapped keys never need
  to be live relay members to import.
- The importer still best-effort registers mapped users as relay members
  (kind `9030`) and channel members (kind `9000`) so their history is
  readable to them the moment they log in with their key.
- A kind `0` profile (display name, avatar URL from `users.json`) is
  published for each mapped user unless `--skip-profiles` is set.

**Key custody warning:** whoever produces `keys.json` holds every mapped
user's private key until it is handed over. Generate keys on one machine,
deliver each `nsec` to its person over a secure channel — Buzz's NIP-AB
pairing (`buzz-pair-relay`) is designed for exactly this one-time key
transfer — and destroy the mapping file after import. Prefer generating the
mapping *with* each user present when the team is small.

### Claim mode (future work)

The zero-custody end state, not yet implemented:

1. Each person onboards in Buzz normally (key generated on-device, never
   leaves it).
2. The importer (as a Slack app) DMs each member a one-time claim token —
   receiving the token proves control of the Slack account; signing the
   claim proves control of the Buzz key. Neither email infrastructure nor
   key distribution is required.
3. Each person runs `buzz import slack --claim <SLACK_ID>` against the
   shared export, signing only their own messages locally.

This needs a shared cross-run message-ID ledger (replies must reference
event IDs of messages signed by *other* users' claims) — the state-file
design below anticipates it, but multi-party coordination is out of scope
for v1. Fallback hierarchy for unclaimed users stays the same: bot-signed
with attribution tags.

## Relay requirements

**The CLI identity must be a community owner or admin.** The relay
normally rejects events whose `created_at` is more than 15 minutes in the
past, and events whose author differs from the authenticated submitter.
Both checks carry an authorized-import exemption: an event with an
`import` provenance tag, submitted by an authenticated community
owner/admin, may be backdated and may be third-party-signed (its Schnorr
signature proves authorship). No relay restart or configuration change is
needed — the operator's own auth *is* the authorization, scoped per event.

Under the hood the exemption also disarms the DB commit-time floor guard
(migration 0021) for exactly those inserts, and on read-replica
deployments it closes the replica fence until a fresh handshake provably
covers the backfilled rows — degraded read capacity during the import,
never missing rows. Single-instance deployments are unaffected.

Two optional knobs for the import window:

```bash
# Only if you cannot grant the importer admin: raise the past-drift window
# relay-wide instead (requires restart; the DB floor guard follows at +60s).
BUZZ_MAX_PAST_DRIFT_SECS=315360000

# Speed: bot mode signs thousands of events with one key and the default
# quota is 60 messages/minute. The importer backs off and retries on 429,
# so an import finishes at default limits — just slowly.
BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN=100000
BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN=100000
```

### Future: self-expiring import window

A possible refinement — proposed, not implemented — is an admin-published
relay command ("open import window: max age N, expires in H hours"),
making the import authorization itself a signed, audit-logged,
self-expiring event, with optional scoping to specific author pubkeys.
The per-event admin exemption above covers the practical cases today.

## Ordering, threads, idempotency

- Channels are imported one at a time; messages within a channel are sorted
  by Slack `ts`, so a thread root is always imported before its replies.
- A state file (default `<export-dir>/buzz-import-state.json`) records
  `slack channel id → Buzz channel UUID` and
  `"<channel>:<ts>" → Nostr event id`. Re-running the import skips
  everything already recorded — interrupted imports resume where they
  stopped. The state file is saved incrementally during the run.
- Reply `e`-tags are resolved from the state map, never from the relay.

## Text conversion (mrkdwn → markdown)

Code blocks and inline code are preserved verbatim. Outside code:

- `<@U123>` → `@DisplayName` (plain text — see mention note below)
- `<#C123|name>` → `#name`
- `<http://url|label>` → `[label](url)`; `<http://url>` → `url`
- `<!here>` / `<!channel>` / `<!everyone>` → `@here` / `@channel` / `@everyone`
- `&lt;` `&gt;` `&amp;` unescaped
- `*bold*` → `**bold**` (conservative, single-line, non-space-adjacent)

**Mentions are intentionally not `p`-tagged.** A `p` tag on thousands of
backdated messages would flood mention feeds and notifications for everyone
who was ever @-mentioned in Slack. Imported mentions render as plain
`@Name` text.

## Limitations (v1)

- **Files are not re-hosted.** Slack file URLs (which require Slack auth)
  are appended as links. A future `--download-files` could fetch blobs with
  a Slack token and re-upload via Blossom, rewriting links.
- **DMs and private channels are not imported.** Standard Slack exports
  only contain public channels; DM import also raises consent questions
  that belong with the claim-mode design.
- **Reaction timestamps are synthetic** (message `ts + 1s`) — Slack exports
  don't record when a reaction was added.
- **Sub-second ordering may flatten.** Two messages inside the same second
  get the same `created_at`; original ordering is preserved in `import_ts`.
- **Edit history is not reconstructed** — the export contains only final
  text.
- **Slack workflows are not translated** to `buzz-workflow` YAML.

## CLI reference

```
buzz import slack
  --export-dir <PATH>      unzipped Slack export directory (required)
  --mapping <PATH>         Slack user id → private key JSON (mapping mode)
  --state <PATH>           state file (default: <export-dir>/buzz-import-state.json)
  --channels <a,b,c>       import only these channel names
  --dry-run                parse and report what would be imported; no writes
  --skip-reactions         do not import reactions
  --skip-profiles          do not publish kind 0 profiles for mapped users
```

Output follows CLI conventions: progress on stderr, a final JSON summary on
stdout (`channels_created`, `messages_imported`, `reactions_imported`,
`skipped`, `warnings`).
