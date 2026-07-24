# Slack Import

`buzz import slack` migrates a Slack workspace export into a Buzz community.
Buzz was built to reduce dependency on Slack; this tool is the on-ramp — it
carries a team's conversational history (channels, messages, threads,
reactions) onto a relay you own, so agents and people can search it as one
record from day one.

```bash
buzz import slack --export-dir ./export --team-id T0266FRGM              # import history
buzz import slack --export-dir ./export --team-id T0266FRGM --dry-run    # plan only
buzz import slack --export-dir ./export --team-id T0266FRGM \
  --identity-map U060=npub1abc,U081=npub1def                            # admin attests people
buzz import bind --team-id T0266FRGM --slack-id U060 --pubkey npub1abc   # admin attests one, later
buzz import claim --team-id T0266FRGM --slack-id U060                    # the person consents (their key)
```

`--team-id` is the Slack workspace id (e.g. `T0266FRGM`). It namespaces every
identity binding and channel UUID so a `U…` id can't collide across two Slack
workspaces.

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
- `["import_author", "<team id>:<user id>", "<display name>"]` — original
  author, workspace-scoped so it composes to the binding key `slack:<team>:<user>`
- `["import_ts", "<slack ts>"]` — original microsecond-precision timestamp
  (Nostr `created_at` is seconds, so this preserves sub-second ordering data)

### Message subtypes

Slack tags non-plain messages with a `subtype`. The importer is deliberately
conservative — it imports genuine conversation and silently skips system noise:

| Slack subtype | Handling |
|---|---|
| *(none)* — a plain message | Imported |
| `thread_broadcast` — a reply also broadcast to the channel | Imported (as a reply) |
| `bot_message` and app/integration posts | Skipped |
| `channel_join` / `channel_leave` / `channel_topic` / `channel_purpose` and other system notices | Skipped |
| `message_changed` (edits) and `message_deleted` / tombstones | Skipped — only final, still-present text is imported; edit history is not reconstructed |
| Any message whose top-level `text` is empty (e.g. Block Kit– or attachment-only posts) | Skipped (see Limitations) |

Timestamps are read as the exact Slack `ts` **string** — integer seconds plus a
microsecond fraction, never parsed as a float. `created_at` is the whole
seconds; the full-precision `ts` is preserved in `import_ts` for ordering.

## Attribution model — zero key custody, two-party consent

Every imported event is signed by the **CLI identity** (bot mode). No
private key is ever generated for, or distributed to, anyone. Message
bodies keep a `**Name**: ` prefix (for search and non-Buzz clients) and the
`import_author` tag records the original person.

Attributing history to a real person takes **two signatures** — an admin
and the person — so neither side can do it alone:

1. **Attestation** (kind `30623`, `KIND_IMPORT_IDENTITY_BINDING`) — a
   community owner/admin signs `slack:<team id>:<user id> → <public key>` (the
   `d` tag is the workspace-scoped `slack:<team>:<user>`). The relay accepts
   this kind only from an owner/admin.
2. **Claim** (kind `30624`, `KIND_IMPORT_IDENTITY_CLAIM`) — the person signs
   their own consent for the same `slack:<team id>:<user id>` with their key. It
   has no `p` tag: the signer *is* the subject, and the relay's signer==author
   rule means you can only ever claim for yourself.

The `d` tag is **workspace-scoped** (`slack:<team>:<user>`) because Slack user
ids are only unique within a workspace. Imported messages carry the same scoped
id in their `import_author` tag, so the client joins a message to its binding
by the identical key. Both events are NIP-33 parameterized-replaceable on that
`d` tag: re-binding supersedes the prior binding, and re-keying or revoking is a
fresh event on the same `d` (a claim can also be replaced by the subject to
withdraw consent).

```bash
# admin attests (npub is public — not a secret):
buzz import slack --export-dir ./export --team-id T0266FRGM --identity-map U060=npub1abc,U081=npub1def
buzz import bind --team-id T0266FRGM --slack-id U060 --pubkey npub1abc   # or one at a time, later

# the person consents, run by them with their own key:
buzz import claim --team-id T0266FRGM --slack-id U060
```

The Buzz client renders imported history under the real person's profile
(name + avatar) **only when both exist and agree** — the attestation's
pubkey equals the claim's signer for the same `slack:<team>:<id>`. Either half
alone is inert; unconfirmed history still shows the `import_author` name.

Why this is safe:

- **Zero end-user key custody.** No member private key is ever generated for
  or distributed to anyone — only public keys (npubs) are handled on the
  attribution path, so there is no `keys.json` of member keys to leak. (The
  optional claim-service does hold the operator's *own* owner/admin key and the
  Slack client secret to sign attestations server-side; that is operator
  infrastructure, not end-user custody — see [Configure the claim
  service](#configure-the-claim-service).)
- **A member can't seize history.** A claim without a matching owner/admin
  attestation attributes nothing — a member cannot map
  `slack:<team>:U060 → their own pubkey` to grab someone else's history.
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
without a Slack-side oracle to prove who really owned `slack:<team>:<id>`; the
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

1. Creates or loads the person's device key, generates a device-held verifier,
   and opens `<service>/oidc/start` with only its one-way challenge.
2. Slack authenticates them through its Sign in with Slack OpenID Connect
   flow. The claim service verifies Slack's signed ID token (issuer, audience,
   expiry, nonce, workspace) and confirms the same identity through Slack
   userInfo.
3. Slack returns through the internal `buzz://import-claim` callback with a
   short-lived finalize code. Buzz verifies that it matches the pending join
   and sends the service both the retained verifier and a self-signed identity
   claim. A stolen custom-scheme callback code is unusable without that
   verifier.
4. The service verifies that signature, idempotently adds the key to the target
   community, and publishes the owner/admin attestation. The code becomes bound
   to that key, so the same device can safely retry a partial network failure
   but another key cannot take over the migration.
5. Buzz connects to the target community and publishes the person's matching
   self-claim. Completing Slack OAuth is the consent action, so there is no
   second confirmation prompt.
6. With both signatures present, imported messages for that Slack user render
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

The Slack exchange uses the documented Sign in with Slack OpenID Connect
endpoints. The device-held verifier described above protects the separate
claim-service-to-Buzz handoff; it is not sent to Slack.

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

When OIDC is enabled, startup logs print the complete
`buzz://join-slack?...` migration link. Share that link with members of the
imported Slack workspace. It is separate from the relay's normal invite link:

- **Imported Slack member:** open the logged migration link; Slack verification
  admits the person's device key and attaches their imported history.
- **Everyone else:** in Buzz, open **Settings → Community access → Create invite
  link**, choose an expiry, and share the resulting HTTPS link or QR code.

> **Don't paste secrets on the command line.** `export BUZZ_PRIVATE_KEY=…` and
> `SLACK_CLIENT_SECRET=…` land in shell history and process listings. Inject
> both from a secret manager (or a `chmod 600` env file loaded by the service
> supervisor) so the owner/admin key and Slack secret never persist in history.
> The two shown here are illustrative.

`SLACK_TEAM_ID` is **required** — it is the `<team>` in every
`slack:<team>:<user>` subject the service mints, on both channels, and (for the
OIDC path) the workspace Slack sign-ins are checked against. `SLACK_CLIENT_ID`
and `SLACK_CLIENT_SECRET` are optional and enable the Sign-in-with-Slack (OIDC)
channel when set together; omit both to run the email channel only.

`--export-dir` supplies `users.json` for the optional email fallback; the OIDC
path gets the verified Slack user id directly from Slack. `--base-url` must be
the externally reachable **HTTPS** claim-service origin, with no path, query,
fragment, or credentials; its `/oidc/callback` is the default OIDC redirect
URI. Plain HTTP is accepted only with `--dev`. Use `--oidc-redirect-uri` only
when the registered redirect differs. If the relay requires a NIP-OA
delegation, also set `BUZZ_AUTH_TAG`.

The join link's `relay` must identify the same community as
`BUZZ_RELAY_URL`. The service's `BUZZ_PRIVATE_KEY` must be an owner/admin key
for that community.

The service holds a Buzz owner/admin private key and the Slack client secret.
Run it as trusted migration infrastructure, keep it off the public relay
process, terminate TLS in front of it, and retire the service and migration
link when onboarding is complete. The join link remains usable while the
service is running; it does not expire by itself. Never run `--dev` in
production. Rate-limit `/oidc/start`, `/oidc/callback`, and `/oidc/finalize` at
the reverse proxy. Pending OIDC state and finalize codes are process-local, so
run one claim-service instance (or use sticky routing); restarting it
invalidates sign-ins currently in progress. Outbound Slack token, userInfo, and
signing-key requests use a 5-second connection timeout and a 30-second total
request timeout.

Before distributing the migration link in production, run one end-to-end smoke
test against the actual Slack application and workspace: open
`buzz://join-slack`, complete Slack authorization, verify that Buzz joins the
expected community, and confirm that imported history resolves to that Buzz
profile. The `--dev` flow exercises the app handoff and relay writes, but it
does not validate Slack application configuration or Slack's live OIDC
responses.

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

> Both knobs are **relay-wide** and both weaken relay protections (past-drift
> anti-spoofing; per-pubkey flood limits) for *every* account, not just the
> importer. Prefer granting the importer owner/admin (the per-event exemption
> above needs no relay-wide change at all). If you must raise them, restore the
> defaults as soon as the import finishes — leaving them at 100000 leaves the
> relay open to backdated-event spoofing and message floods.

### Future: self-expiring import window

A possible refinement — proposed, not implemented — is an admin-published
relay command ("open import window: max age N, expires in H hours"),
making the import authorization itself a signed, audit-logged,
self-expiring event, with optional scoping to specific author pubkeys.
The per-event admin exemption above covers the practical cases today.

## Ordering, threads, idempotency

- Channels are imported one at a time; messages within a channel are sorted
  by Slack `ts`, so a thread root is always imported before its replies. If a
  thread root is itself skipped (an empty `bot_message`, a Block Kit–only post),
  its replies can't be `e`-tagged to it, so they are imported as ordinary
  top-level messages instead of being dropped — their real content is preserved,
  only the (contentless) thread linkage is lost. A warning is logged per reply.
- A state file (default `<export-dir>/buzz-import-state.json`) records
  `slack channel id → Buzz channel UUID` and
  `"<channel>:<ts>" → Nostr event id`. Re-running the import skips
  everything already recorded — interrupted imports resume where they
  stopped. The state file is saved incrementally during the run.
- The state file is pinned to its schema version and `SLACK_TEAM_ID`. Reusing
  it with another workspace is rejected. A non-empty state created before
  workspace-scoped identities is also rejected because its skipped messages
  could carry incompatible `import_author` values.
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
- **Per-reactor identity in reactions is not preserved.** Bot mode signs one
  reaction per distinct emoji — a single key can react to a target once *per
  emoji* (NIP-25), so N reactors of the same emoji collapse to one event and the
  reactor count is lost.
- **Reaction timestamps are synthetic** (message `ts + 1s`) — Slack exports
  don't record when a reaction was added.
- **Sub-second ordering may flatten.** Two messages inside the same second
  get the same `created_at`; original ordering is preserved in `import_ts`.
- **Edit history is not reconstructed** — the export contains only final
  text.
- **Block Kit / attachment-only messages are dropped.** A message whose
  top-level `text` is empty (rich content lives only in `blocks`/`attachments`)
  carries no plain text to import and is skipped. Bot/app posts are skipped for
  the same reason plus their `bot_message` subtype. Replies **to** such a skipped
  root are not lost — they import as top-level messages (see Ordering, threads,
  idempotency).
- **Slack workflows are not translated** to `buzz-workflow` YAML.

## CLI reference

```
buzz import slack
  --export-dir <PATH>      unzipped Slack export directory (required)
  --team-id <ID>           Slack workspace id (required, e.g. T0266FRGM)
  --state <PATH>           state file (default: <export-dir>/buzz-import-state.json)
  --channels <a,b,c>       import only these channel names
  --dry-run                parse and report what would be imported; no writes
  --skip-reactions         do not import reactions
  --identity-map <MAP>     SLACKID=npub-or-hex,… admin attestations (public keys only)

buzz import bind           # owner/admin half: attest a Slack id → public key
  --team-id <ID>           Slack workspace id (e.g. T0266FRGM)
  --slack-id <ID>          Slack user id (e.g. U060976D0QN)
  --pubkey <NPUB|HEX>      the person's PUBLIC key (never an nsec)

buzz import claim          # subject half: consent, run by the person with their key
  --team-id <ID>           Slack workspace id (e.g. T0266FRGM)
  --slack-id <ID>          your Slack user id (e.g. U060976D0QN)
```

Output follows CLI conventions: progress on stderr, a final JSON summary on
stdout (`channels_created`, `messages_imported`, `reactions_imported`,
`bindings_published`, `skipped`, `warnings`).
