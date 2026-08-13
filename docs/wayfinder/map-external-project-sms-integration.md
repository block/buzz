# Buzz External Project Integration + SMS Operator Agent

## Goal

Let a message arriving in Buzz (from a channel or, eventually, an SMS) trigger an agent dispatch that operates against an *external* repo (bidcraft/BuildBid, construct-pro) instead of only the harness's own working directory. Reuse Buzz's existing NIP-MP `project` primitive for repo scoping rather than inventing a new one, and add a Twilio SMS front door that resolves to the same dispatch path once messages land in Buzz as normal channel events.

**Note:** `mfethe1/bidcraft` (GitHub) is the same project as BuildBid — a construction-estimating SaaS (AI vision/estimation pipeline, Supabase auth, Stripe billing, deployed on Railway, live at buildbid.app). The two names refer to one project.

**Corrected local path (verified 2026-08-12):** `E:/Projects/buildbid` itself is NOT a git checkout — it's a parent folder holding many separate clones/worktrees (`bidcraft-repo`, `bidcraft-repo-claude2`, `bidcraft-cruft-cleanup`, etc.), each with its own `.git`. The canonical checkout is **`E:/Projects/buildbid/bidcraft-repo`** (`origin` = `git@github.com:mfethe1/bidcraft.git`, confirmed via `git remote -v`). Any `--project-paths` entry for `bidcraft` must point here, not at the bare `E:/Projects/buildbid` folder.

## Privacy (bidcraft/construct-pro are private repos — verified 2026-08-12)

Nothing in this design requires making either repo public. Repo *content* never
needs to transit a public surface — the harness just points an agent's working
directory at your existing private local clone. But two Buzz primitives used
by this design default to discoverable, and must be overridden explicitly:

1. **NIP-MP project visibility defaults to `listed`.** `ProjectVisibility::Listed`
   is documented as "Project appears in public listings **(default)**"
   (`crates/buzz-cli/src/lib.rs:1262-1263`). Creating a `bidcraft` or
   `construct-pro` project via `buzz projects create`/`update` without
   `--visibility unlisted` makes the *existence* of that project binding
   discoverable to anyone who can browse projects on the relay — not the
   code, but the fact that you have a project by that name wired up.
   **Action: always pass `--visibility unlisted` when creating or updating
   the bidcraft/construct-pro project bindings.**
2. **Channel visibility defaults to `open`.** `default_visibility()` in
   `crates/buzz-cli/src/commands/channel_templates.rs:60` returns `"open"`
   — "searchable, anyone can join without an invite"
   (`ChannelVisibility::Open`, `crates/buzz-core/src/channel.rs:20-27`), vs.
   `Private` ("hidden, requires an invite"). Any channel bound to bidcraft/
   construct-pro, and the future SMS-inbox channel, **must be created with
   `visibility: private`**, not left at the open default.
3. **"Unlisted"/"private" are discoverability controls, not encryption.**
   Events are still stored plaintext in the relay's own database. The real
   privacy boundary is who can authenticate to *this* relay/community at
   all (NIP-42 auth, `require_auth_token`, community host-scoping) — these
   two settings only prevent casual browse-discovery by users who already
   have access to the relay, they don't add a second layer of access
   control on top of it.

**Latest code, not a stale snapshot:** dispatch already points at your live
local checkout, so it always sees whatever's currently on disk. Nothing here
forks or mirrors the repo into a Buzz-controlled copy. The one open gap:
`resolve_effective_cwd`/`build_channel_cwd_map` (`crates/buzz-acp/src/pool.rs`,
`config.rs`) don't currently `git fetch`/pull before dispatch, so a checkout
that's fallen behind its remote (e.g. someone else pushed) won't be
automatically refreshed — tracked as a new task (see plan below).

**New capability lands in Buzz itself:** the project→path and channel→project
config added in slices 1+2 is first-class `buzz-acp`/`buzz-cli` config, not a
one-off script bolted on the side — any future project gets the same
treatment for free.

## Part A: Buzz ↔ bidcraft/construct-pro agent dispatch

**Project representation — reuse `KIND_PROJECT` (30621, NIP-MP), don't overload community.** Community is the relay-tenant boundary (one relay/host); project is already the right-sized primitive for "which repo(s) does this dispatch target" — it groups `kind:30617` git-repo announcements via `a`-tag coordinates and already has CLI surface (`buzz-cli/src/lib.rs:215-220`, `projects`/`repos` subcommands). No new event kind needed for project scoping itself.

**New/changed pieces:**

1. **`cwd` plumbing (the actual gap).** `AcpClient::session_new_full` already accepts an arbitrary absolute `cwd` per the ACP wire protocol (`acp.rs:621`, `acp.rs:638`), but every caller hardcodes `std::env::current_dir()` (`lib.rs:1600`, `lib.rs:4149`). Add `cwd: Option<String>` resolution to the prompt-dispatch path in `lib.rs`: if the triggering event (or its channel's persona config) carries a project reference, resolve it to a local worktree path and pass that; otherwise fall back to current behavior. Validate the resolved path is absolute and exists before passing it into `session_new_full` — don't trust a tag value directly into a filesystem path (untrusted input → path traversal risk).
2. **Project → local-path resolution.** ✅ Done (`--project-paths` CLI/env config, `crates/buzz-acp/src/config.rs`) mapping a project's `d`-tag identifier to a local checkout path, e.g. `bidcraft` → `E:/Projects/buildbid/bidcraft-repo`, `construct-pro` → a `.claude/worktrees/buzz-<slug>` path (avoid the taken `mack/`, `honey/`, `winnie/`, `airy/`, `fizz/`, `parity/`, `dispatch-` namespaces already in use in construct-pro). This is harness-side config, not new protocol — the mapping doesn't need to live on-relay.
3. **Tagging convention.** Channel-message events that should dispatch against an external project carry an `a`-tag pointing at the `kind:30621` project coordinate (NIP-MP's existing addressing), or the channel's own persona/config is statically bound to one project. Prefer the static binding for v1 (simpler, matches "one operator channel per external project" from Part B) and treat per-message `a`-tag override as a stretch goal.
4. **`buzz-workflow` / `buzz-cli` changes.** No `buzz-workflow` changes required for this part — dispatch already happens through `buzz-acp`'s own event loop, not through workflow webhooks. ~~`buzz-cli` needs one addition...~~ **Correction (verified 2026-08-12): no new `buzz-cli` subcommand needed.** `buzz projects update <slug> --channel <uuid>` already writes a `["buzz-channel", uuid]` tag onto the kind:30621 project event (`buzz-cli/src/commands/projects.rs:477-478`, with an existing unit test asserting exactly-one-tag replace semantics at `projects.rs:921-941`), and `projects get`/`list` already print the raw event JSON including that tag. The relay-side channel↔project binding surface already existed; only the harness-local project-id→filesystem-path layer (`--project-paths`, slice 1) and the harness-local channel-id→project-id layer (`--channel-projects`, slice 2 — kept as a separate static flag rather than fetched from the relay, since it's simpler for v1 and avoids a relay dependency at harness startup) were actually missing.
5. **Worktree hygiene for construct-pro specifically.** New worktrees go under `.claude/worktrees/buzz-<slug>` in construct-pro, never reusing an active agent's handle prefix.
6. **bidcraft/BuildBid specifically.** ✅ Verified — the canonical checkout is `E:/Projects/buildbid/bidcraft-repo` (not the bare `E:/Projects/buildbid` folder, which has no top-level `.git` and just holds many separate clones). Confirmed real: `origin` = `git@github.com:mfethe1/bidcraft.git`, real commit history, currently on branch `design/tokens-refresh-2026-08-01` with a couple untracked files (pre-existing, not from this work).

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
- **BuildBid/bidcraft local checkout state** — ✅ resolved: use `E:/Projects/buildbid/bidcraft-repo`, not the bare `E:/Projects/buildbid` folder.
- **Live dispatch testing (slices 4, 5, 11)** — these need a running relay + a real `buzz-acp` harness process + a real agent turn. That's a meaningfully bigger and more consequential action than a code change (real subprocess execution, real writes into shared repos other agents are actively working in, particularly construct-pro). Recommend running these as a supervised session rather than unattended background execution.
- **construct-pro worktree slot** — needs a decision on which unclaimed namespace (`buzz-<slug>`) to standardize on, and whether Buzz dispatch should target the open `map.md` WF-01 queue as its first live test.

## Phased implementation plan (vertical slices)

1. **Project→path config resolution (no dispatch yet)** → verify: add a `[projects]` mapping entry in harness config pointing a fake project id at a scratch directory; write a unit test that `config.rs`'s resolver returns the expected path for a known id and `None` for unknown.
2. **Thread `cwd` through `session_new_full` callers** → verify: manually trigger a channel message with a bound project, observe (via ACP subprocess launch args/log) that the spawned agent's working directory is the resolved external path, not `std::env::current_dir()`; confirm a message on an unbound channel still uses the old default (regression check).
3. **`buzz-cli` project-binding subcommand** → verify: run the new subcommand against a running relay, confirm it reads/writes the channel↔project binding and that a subsequent dispatch (slice 2) picks it up without a harness restart, if config is live-reloaded — otherwise document the restart requirement.
4. **First live external dispatch — bidcraft/BuildBid (read-only)** → verify: bind a test channel to bidcraft's project id (`E:/Projects/buildbid/bidcraft-repo`), post a message asking the agent to summarize open work, confirm the response reflects real repo state. **Deferred pending a supervised session** — needs a running relay + real harness process, a bigger step than a code change.
5. **First live external dispatch — construct-pro (code change)** → verify: bind a channel to construct-pro, dispatch against WF-01 (open, unblocked, smallest ticket per `docs/wayfinder/map.md`), confirm the agent works inside a fresh `.claude/worktrees/buzz-<slug>` worktree and produces a diff addressing the ticket's cited evidence lines. **Deferred pending a supervised session** — this dispatches a real agent that writes into a shared, actively-worked repo (multiple other agents have branches in flight there); not something to run unattended.
6. **`sms_identities` migration + allow-list enforcement** ✅ done — `migrations/0031_sms_identities.sql`, `crates/buzz-db/src/sms.rs`. Not yet applied against a live Postgres (no infra spun up this pass).
7. **Twilio signature validation + inbound webhook route** ✅ done — `crates/buzz-relay/src/twilio_auth.rs` (HMAC-SHA1 validation, test vectors independently cross-checked via `openssl` and Python outside this codebase), `crates/buzz-relay/src/api/sms.rs` wired at `POST /hooks/sms/inbound`. Verified: unit tests pass, `cargo clippy -D warnings` clean. **Stops at "allowed → 200 OK acknowledged"** — does not yet synthesize a `KIND_STREAM_MESSAGE_V2` event; that lands with slice 8.
8. **Operator persona — fast path (default_project set)** → verify: seed an `sms_identities` row with `default_project` = bidcraft or construct-pro, post a synthetic inbound-SMS event into the SMS-inbox channel, confirm the persona dispatches via the Part A path (slice 2) into the correct project's `cwd`. Also where `sms.rs`'s handler grows the actual `KIND_STREAM_MESSAGE_V2` event synthesis it currently stops short of.
9. **Operator persona — ambiguous path** → verify: seed a row with `default_project = NULL`, post an inbound event, confirm the persona posts a disambiguation reply event instead of dispatching.
10. **Outbound SMS sink** → verify: with real (or Twilio test-credential sandbox) SID/token configured, post a reply event tagged back to an inbound message, confirm `sms_sink.rs` calls Twilio's API and the test phone number (or Twilio's magic test number) receives/logs the outbound message.
11. **End-to-end SMS → project dispatch → reply** → verify: full loop — real inbound SMS from an allow-listed number with a clear `default_project`, agent dispatches and completes, outbound reply SMS arrives back at the sending number.
12. **Fetch-before-dispatch freshness check (new — from privacy/freshness review)** → verify: point `--project-paths` at a checkout that's behind its remote, dispatch into it, confirm the harness either fast-forwards it first or at minimum logs a clear staleness warning rather than silently working off outdated code.

## Open questions for Michael

- Should `linked_pubkey` be required for `allowed = true` rows, or is the shared "SMS relay" service pubkey acceptable for allow-listed-but-unlinked numbers?
- Confirm Buzz dispatch's first live test target: construct-pro's `map.md` WF-01 (open, unblocked, small), or a Buzz-specific ticket filed in that same queue instead?
- Do you already have a Twilio account and a number, or does that need to be set up before slice 6 can be tested against real signatures (vs. a stubbed/test AuthToken)?
- ~~Is `E:/Projects/buildbid` the real checkout for `mfethe1/bidcraft`~~ — resolved: it's `E:/Projects/buildbid/bidcraft-repo`.
- Which `.claude/worktrees/buzz-<slug>` name to use for the construct-pro live dispatch test, and whether to run slices 4/5/11 (live dispatch) as a supervised session rather than unattended.

---
Sources: buzz-external-integration-design workflow (5 research agents), run 2026-08-12
Last updated: 2026-08-12
