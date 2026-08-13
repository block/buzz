# Buzz External Project Integration + SMS Operator Agent

## Goal

Let a message arriving in Buzz (from a channel or, eventually, an SMS) trigger an agent dispatch that operates against an *external* repo (bidcraft/BuildBid, construct-pro) instead of only the harness's own working directory. Reuse Buzz's existing NIP-MP `project` primitive for repo scoping rather than inventing a new one, and add a Twilio SMS front door that resolves to the same dispatch path once messages land in Buzz as normal channel events.

**Note:** `mfethe1/bidcraft` (GitHub) is the same project as the local `E:\Projects\buildbid` checkout — it's BuildBid, a construction-estimating SaaS (AI vision/estimation pipeline, Supabase auth, Stripe billing, deployed on Railway, live at buildbid.app). The two names refer to one project.

## Part A: Buzz ↔ bidcraft/construct-pro agent dispatch

**Project representation — reuse `KIND_PROJECT` (30621, NIP-MP), don't overload community.** Community is the relay-tenant boundary (one relay/host); project is already the right-sized primitive for "which repo(s) does this dispatch target" — it groups `kind:30617` git-repo announcements via `a`-tag coordinates and already has CLI surface (`buzz-cli/src/lib.rs:215-220`, `projects`/`repos` subcommands). No new event kind needed for project scoping itself.

**New/changed pieces:**

1. **`cwd` plumbing (the actual gap).** `AcpClient::session_new_full` already accepts an arbitrary absolute `cwd` per the ACP wire protocol (`acp.rs:621`, `acp.rs:638`), but every caller hardcodes `std::env::current_dir()` (`lib.rs:1600`, `lib.rs:4149`). Add `cwd: Option<String>` resolution to the prompt-dispatch path in `lib.rs`: if the triggering event (or its channel's persona config) carries a project reference, resolve it to a local worktree path and pass that; otherwise fall back to current behavior. Validate the resolved path is absolute and exists before passing it into `session_new_full` — don't trust a tag value directly into a filesystem path (untrusted input → path traversal risk).
2. **Project → local-path resolution.** Add a config table/section (in `config.rs`, near `resolve_channel_filters`, config.rs:1241) mapping a project's `d`-tag identifier to a local checkout path, e.g. `bidcraft` → `E:/Projects/buildbid`, `construct-pro` → a `.claude/worktrees/buzz-<slug>` path (avoid the taken `mack/`, `honey/`, `winnie/`, `airy/`, `fizz/`, `parity/`, `dispatch-` namespaces already in use in construct-pro). This is harness-side config, not new protocol — the mapping doesn't need to live on-relay.
3. **Tagging convention.** Channel-message events that should dispatch against an external project carry an `a`-tag pointing at the `kind:30621` project coordinate (NIP-MP's existing addressing), or the channel's own persona/config is statically bound to one project. Prefer the static binding for v1 (simpler, matches "one operator channel per external project" from Part B) and treat per-message `a`-tag override as a stretch goal.
4. **`buzz-workflow` / `buzz-cli` changes.** No `buzz-workflow` changes required for this part — dispatch already happens through `buzz-acp`'s own event loop, not through workflow webhooks. `buzz-cli` needs one addition per AGENTS.md's "agent-facing operations go in `buzz-cli`" convention: a subcommand to inspect/set a channel's bound project (thin wrapper over the existing `projects`/`repos` subcommands, `buzz-cli/src/lib.rs:215-220`), so an operator can configure the binding without hand-editing harness config.
5. **Worktree hygiene for construct-pro specifically.** New worktrees go under `.claude/worktrees/buzz-<slug>` in construct-pro, never reusing an active agent's handle prefix.
6. **bidcraft/BuildBid specifically.** No clone needed — it's already local at `E:/Projects/buildbid`. Confirm it's a valid git checkout before binding it as a dispatch target (an earlier check found no `.git` at the top level; may need `git init`/re-clone or point at the actual nested repo root).

**Files:** `crates/buzz-acp/src/{lib.rs,acp.rs,config.rs}`, `crates/buzz-cli/src/lib.rs:212-220`, `crates/buzz-core/src/kind.rs` (no change — reusing 30621), new config section for project→path mapping.

## Part B: Twilio SMS + Operator Agent

1. **Inbound webhook — dedicated route, not `/hooks/{id}`.** New `crates/buzz-relay/src/api/sms.rs`, wired at `POST /hooks/sms/inbound` in `router.rs` (alongside router.rs:121). Twilio's form-encoded body (`From`, `Body`, `To`, `MessageSid`) needs its own extractor — `buzz-workflow`'s `Webhook` trigger (`bridge.rs:1800`) is bound to a specific pre-authored `WorkflowDef` UUID and doesn't fit "arbitrary inbound SMS → new event."
2. **Signature validation.** New `crates/buzz-relay/src/twilio_auth.rs` implementing Twilio's HMAC-SHA1(URL + sorted params, AuthToken) scheme — this is the sole trust boundary, since Twilio can't hold a Nostr key and NIP-42/NIP-98 don't apply.
3. **Phone allow-list + project-default mapping.** New migration:
   ```sql
   CREATE TABLE sms_identities (
     phone_number    TEXT PRIMARY KEY,   -- E.164
     community_id    UUID NOT NULL REFERENCES communities(id),
     allowed         BOOLEAN NOT NULL DEFAULT false,
     linked_pubkey   BYTEA,              -- 32-byte pubkey, nullable
     default_project TEXT,               -- NIP-MP project d-tag, e.g. "bidcraft" | "construct-pro" | NULL
     created_at, updated_at
   );
   ```
   Enforced inside `sms.rs` before synthesizing any event: missing row or `allowed = false` → 403, no event, no reply (closes the spam/oracle vector). `default_project` is a string identifier matching a `kind:30621` project's `d`-tag — this is Part A's primitive reused, not a new scoping concept.
4. **Synthesized event.** Reuse `KIND_STREAM_MESSAGE_V2` (40002, `kind.rs:468`) — no new kind. Author = `linked_pubkey` if set, else a per-community "SMS relay" service pubkey. Tags: `["sms_from", phone_number]`, `["sms_sid", MessageSid]`, `h` = the community's fixed SMS-inbox channel group id.
5. **Operator persona — decision logic.** New `crates/buzz-persona/sms-operator.toml`, subscribed only to the SMS-inbox channel (`h`-tag filter):
   - Fast path: `default_project` on the `sms_identities` row resolves directly → dispatch via Part A's `cwd`-resolution path, scoped to that project.
   - Ambiguous (no default, or content contradicts default): persona posts a reply event in SMS-inbox ("Reply 1 for bidcraft, 2 for construct-pro"), which flows out through the outbound sink (step 6) as a real SMS; the next inbound reply resolves it.
   - Once resolved, dispatch is a normal `buzz-acp` `session_new_full` call — no relay-side change beyond the event plumbing above.
6. **Outbound SMS sink.** New `crates/buzz-relay/src/sms_sink.rs`, mirroring `workflow_sink.rs`'s pattern: subscribes server-side to the SMS-inbox channel, on any reply event tagged (`e`-tag) back to an inbound `sms_from` message, calls Twilio's `POST /Messages`.
7. **Secrets.** Twilio Account SID + Auth Token as relay config/env secrets — never in the `sms_identities` table.

**Files:** `crates/buzz-relay/src/api/sms.rs`, `crates/buzz-relay/src/twilio_auth.rs`, `crates/buzz-relay/src/sms_sink.rs`, `crates/buzz-relay/src/router.rs`, `migrations/00XX_sms_identities.sql`, `crates/buzz-persona/sms-operator.toml`.

## Dependencies / things only the user can provide

- **Twilio account + AuthToken/Account SID** — needed before `twilio_auth.rs` can be tested against real signatures.
- **A purchased/rented Twilio phone number** — ~$1/mo recurring, required for both inbound routing and outbound sends; not auto-provisionable.
- **Per-message SMS cost** — roughly $0.0079+/message in the US, scales with volume; ongoing operating cost, not a one-time build cost.
- **BuildBid/bidcraft local checkout state** — confirm `E:/Projects/buildbid` is a proper git checkout (or point at wherever its actual `.git` root is) before binding it as a dispatch target.
- **construct-pro worktree slot** — needs a decision on which unclaimed namespace (`buzz-<slug>`) to standardize on, and whether Buzz dispatch should target the open `map.md` WF-01 queue as its first live test.

## Phased implementation plan (vertical slices)

1. **Project→path config resolution (no dispatch yet)** → verify: add a `[projects]` mapping entry in harness config pointing a fake project id at a scratch directory; write a unit test that `config.rs`'s resolver returns the expected path for a known id and `None` for unknown.
2. **Thread `cwd` through `session_new_full` callers** → verify: manually trigger a channel message with a bound project, observe (via ACP subprocess launch args/log) that the spawned agent's working directory is the resolved external path, not `std::env::current_dir()`; confirm a message on an unbound channel still uses the old default (regression check).
3. **`buzz-cli` project-binding subcommand** → verify: run the new subcommand against a running relay, confirm it reads/writes the channel↔project binding and that a subsequent dispatch (slice 2) picks it up without a harness restart, if config is live-reloaded — otherwise document the restart requirement.
4. **First live external dispatch — bidcraft/BuildBid (read-only)** → verify: bind a test channel to bidcraft's project id, post a message asking the agent to summarize open work in `E:/Projects/buildbid`, confirm the response reflects real repo state.
5. **First live external dispatch — construct-pro (code change)** → verify: bind a channel to construct-pro, dispatch against WF-01 (open, unblocked, smallest ticket per `docs/wayfinder/map.md`), confirm the agent works inside a fresh `.claude/worktrees/buzz-<slug>` worktree and produces a diff addressing the ticket's cited evidence lines.
6. **`sms_identities` migration + allow-list enforcement** → verify: run the migration locally, hit `/hooks/sms/inbound` with a forged Twilio-shaped POST for an unlisted number, confirm 403 and no event written; add an allowed row and confirm the same request now produces a `KIND_STREAM_MESSAGE_V2` event with correct tags.
7. **Twilio signature validation** → verify: send a request with a wrong/missing `X-Twilio-Signature`, confirm 403; send one with a correctly computed signature (using a test Auth Token), confirm it passes and reaches the allow-list check.
8. **Operator persona — fast path (default_project set)** → verify: seed an `sms_identities` row with `default_project` = bidcraft or construct-pro, post a synthetic inbound-SMS event into the SMS-inbox channel, confirm the persona dispatches via the Part A path (slice 2) into the correct project's `cwd`.
9. **Operator persona — ambiguous path** → verify: seed a row with `default_project = NULL`, post an inbound event, confirm the persona posts a disambiguation reply event instead of dispatching.
10. **Outbound SMS sink** → verify: with real (or Twilio test-credential sandbox) SID/token configured, post a reply event tagged back to an inbound message, confirm `sms_sink.rs` calls Twilio's API and the test phone number (or Twilio's magic test number) receives/logs the outbound message.
11. **End-to-end SMS → project dispatch → reply** → verify: full loop — real inbound SMS from an allow-listed number with a clear `default_project`, agent dispatches and completes, outbound reply SMS arrives back at the sending number.

## Open questions for Michael

- Should `linked_pubkey` be required for `allowed = true` rows, or is the shared "SMS relay" service pubkey acceptable for allow-listed-but-unlinked numbers?
- Confirm Buzz dispatch's first live test target: construct-pro's `map.md` WF-01 (open, unblocked, small), or a Buzz-specific ticket filed in that same queue instead?
- Do you already have a Twilio account and a number, or does that need to be set up before slice 6 can be tested against real signatures (vs. a stubbed/test AuthToken)?
- Is `E:/Projects/buildbid` the real checkout for `mfethe1/bidcraft`, or a separate/stale local copy that should be re-pointed at the actual repo?

---
Sources: buzz-external-integration-design workflow (5 research agents), run 2026-08-12
Last updated: 2026-08-12
