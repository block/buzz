# Slack Import

`buzz import slack` migrates a Slack workspace export into a Buzz community.
Buzz was built to reduce dependency on Slack; this tool is the on-ramp — it
carries a team's conversational history (channels, messages, threads,
reactions) onto a relay you own, so agents and people can search it as one
record from day one.

```bash
buzz import slack --export-dir ./my-workspace-export            # import history
buzz import slack --export-dir ./export --dry-run              # plan only
buzz import slack --export-dir ./export \
  --identity-map U060=npub1abc,U081=npub1def                    # admin attests people
buzz import bind --slack-id U060 --pubkey npub1abc              # admin attests one, later
buzz import claim --slack-id U060                               # the person consents (their key)
```

## What gets imported

| Slack | Buzz | Notes |
|-------|------|-------|
| Channels (`channels.json`) | kind `9007` create + `9002` topic/purpose | UUID generated per channel, recorded in the state file |
| Messages (per-day JSON) | kind `9` stream message, `h`-tagged | `created_at` backdated to the original Slack `ts` |
| Threads (`thread_ts`) | NIP-10 `e` reply tags | Slack threads are flat; every reply is a direct reply to the root |
| Reactions | kind `7` | One bot-signed reaction per distinct emoji (per-reactor identity isn't reproduced) |
| Files | Links appended to message content | Blobs are **not** downloaded/re-hosted (see Limitations) |
| Custom emoji | — | Use `scripts/grab-emoji.sh` (separate tool, needs a Slack API token) |

Every imported event carries provenance tags:

- `["import", "slack"]` — marks the event as imported (the `<source>`)
- `["import_author", "<slack user id>", "<display name>"]` — original author
- `["import_ts", "<slack ts>"]` — original microsecond-precision timestamp
  (Nostr `created_at` is seconds, so this preserves sub-second ordering data)

## Attribution model — zero key custody, two-party consent

Every imported event is signed by the **CLI identity** (bot mode). No
private key is ever generated for, or distributed to, anyone. Message
bodies keep a `**Name**: ` prefix (for search and non-Buzz clients) and the
`import_author` tag records the original person.

Attributing history to a real person takes **two signatures** — an admin
and the person — so neither side can do it alone:

1. **Attestation** (kind `30623`, `KIND_IMPORT_IDENTITY_BINDING`) — a
   community owner/admin signs `slack:<user id> → <public key>`. The relay
   accepts this kind only from an owner/admin.
2. **Claim** (kind `30624`, `KIND_IMPORT_IDENTITY_CLAIM`) — the person signs
   their own consent for the same `slack:<user id>` with their key. It has
   no `p` tag: the signer *is* the subject, and the relay's signer==author
   rule means you can only ever claim for yourself.

```bash
# admin attests (npub is public — not a secret):
buzz import slack --export-dir ./export --identity-map U060=npub1abc,U081=npub1def
buzz import bind --slack-id U060 --pubkey npub1abc      # or one at a time, later

# the person consents, run by them with their own key:
buzz import claim --slack-id U060
```

The Buzz client renders imported history under the real person's profile
(name + avatar) **only when both exist and agree** — the attestation's
pubkey equals the claim's signer for the same `slack:<id>`. Either half
alone is inert; unconfirmed history still shows the `import_author` name.

Why this is safe:

- **Zero custody.** Only public keys (npubs) are handled. Nothing secret is
  generated or distributed, so there is no `keys.json` to leak.
- **A member can't seize history.** A claim without a matching owner/admin
  attestation attributes nothing — a member cannot map
  `slack:U060 → their own pubkey` to grab someone else's history.
- **An admin can't forge authorship.** An attestation without the subject's
  own claim attributes nothing either — an admin cannot make an existing
  member appear to have written imported messages. This is the vector a
  single admin signature could not close.
- **No third-party signatures.** Every stored event is signed by its
  submitter; there is no path for one key to post as another.
- Display names remain freely editable (Slack-like); the binding ties a
  Slack id to a **pubkey**, independent of display name. Verified handles
  are a separate layer (NIP-05).

**Residual trust.** A *colluding* owner/admin and a consenting pubkey can
still attribute orphaned history to that pubkey (both sign). This is inherent
without a Slack-side oracle to prove who really owned `slack:<id>`; the
consenting party is publicly volunteering, and the immutable `import_author`
provenance on every event records the original Slack identity regardless.
What two-party consent removes is the *unilateral* admin — the realistic
insider risk before production.

### Slack migration join

For people coming from the imported Slack workspace, use one dedicated
onboarding link:

```text
buzz://join-slack?relay=<percent-encoded-ws(s)-relay>&service=<percent-encoded-https-claim-service>
```

For example:

```text
buzz://join-slack?relay=wss%3A%2F%2Fbuzz.example.com&service=https%3A%2F%2Fmigrate.example.com
```

Opening it in Buzz:

1. Creates or loads the person's device key and opens
   `<service>/oidc/start` in their browser.
2. Slack authenticates them. The claim service rejects an account from any
   workspace other than its configured `SLACK_TEAM_ID`.
3. The service idempotently adds that device's public key to the target
   community and publishes the owner/admin attestation.
4. Slack returns through the internal `buzz://import-claim` callback. Buzz
   asks the person to confirm the link, connects to the target community, and
   only then publishes their self-signed claim.
5. With both signatures present, imported messages for that Slack user render
   under the person's Buzz profile.

Slack OAuth is intentionally a migration-time onboarding method. It is not a
permanent Buzz sign-in method and does not appear in Settings. Do not
distribute `buzz://import-claim` URLs: they are short-lived callbacks generated
by the claim service, not an alternative invitation format. Use normal Buzz
invite links for people who are not members of the imported Slack workspace.
The `buzz import bind` and `buzz import claim` commands above remain
manual fallbacks.

#### Configure the claim service

Create a Slack OIDC application for the workspace and register this exact
redirect URL:

```text
https://migrate.example.com/oidc/callback
```

Run `buzz-migrate` behind HTTPS with an owner/admin Buzz key:

```bash
export BUZZ_RELAY_URL=wss://buzz.example.com
export BUZZ_PRIVATE_KEY=<owner-or-admin-nsec-or-hex>
export SLACK_CLIENT_ID=<slack-client-id>
export SLACK_CLIENT_SECRET=<slack-client-secret>
export SLACK_TEAM_ID=<slack-workspace-id>

buzz-migrate \
  --export-dir ./my-workspace-export \
  --bind 127.0.0.1:8787 \
  --base-url https://migrate.example.com
```

`--export-dir` supplies `users.json` for the optional email fallback; the OIDC
path gets the verified Slack user id directly from Slack. `--base-url` must be
the externally reachable claim-service origin; its `/oidc/callback` is the
default OIDC redirect URI. Use `--oidc-redirect-uri` only when the registered
redirect differs. If the relay requires a NIP-OA delegation, also set
`BUZZ_AUTH_TAG`.

The join link's `relay` must identify the same community as
`BUZZ_RELAY_URL`. The service's `BUZZ_PRIVATE_KEY` must be an owner/admin key
for that community.

The service holds a Buzz owner/admin private key and the Slack client secret.
Run it as trusted migration infrastructure, keep it off the public relay
process, terminate TLS in front of it, and retire the service and migration
link when onboarding is complete. The join link remains usable while the
service is running; it does not expire by itself. Never run `--dev` in
production.

The email magic-link channel is an identity-attribution fallback for someone
who is already a community member; it deliberately does **not** grant
membership. Only a workspace-verified Slack OIDC join performs automatic
member admission. `buzz-migrate` currently has no production email-delivery
backend, so this channel is only useful in `--dev` or after integrating a
mailer.

## Relay requirements

**The CLI identity must be a community owner or admin.** Two relay checks
matter for imports:

- Backdating: the relay rejects `created_at` more than 15 minutes in the
  past, *except* for `import`-tagged events submitted by an owner/admin —
  no restart or config change; the operator's auth is the authorization,
  scoped per event. (Under the hood this also disarms the DB commit-time
  floor guard for those inserts, and on read-replica deployments closes the
  replica fence until a fresh handshake covers the backfilled rows —
  degraded read capacity during the import, never missing rows.
  Single-instance deployments are unaffected.)
- Identity **attestations** (kind `30623`): accepted **only** from an
  owner/admin. Identity **claims** (kind `30624`) are self-signed and
  accepted from any member — but only for their own pubkey, and inert without
  a matching attestation.

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
  only contain public channels.
- **Per-reactor identity in reactions is not preserved.** Bot mode signs
  one reaction per distinct emoji (a key can react to a target only once).
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
  --state <PATH>           state file (default: <export-dir>/buzz-import-state.json)
  --channels <a,b,c>       import only these channel names
  --dry-run                parse and report what would be imported; no writes
  --skip-reactions         do not import reactions
  --identity-map <MAP>     SLACKID=npub-or-hex,… admin attestations (public keys only)

buzz import bind           # owner/admin half: attest a Slack id → public key
  --slack-id <ID>          Slack user id (e.g. U060976D0QN)
  --pubkey <NPUB|HEX>      the person's PUBLIC key (never an nsec)

buzz import claim          # subject half: consent, run by the person with their key
  --slack-id <ID>          your Slack user id (e.g. U060976D0QN)
```

Output follows CLI conventions: progress on stderr, a final JSON summary on
stdout (`channels_created`, `messages_imported`, `reactions_imported`,
`bindings_published`, `skipped`, `warnings`).
