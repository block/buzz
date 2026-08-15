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
6. **`sms_identities` migration + allow-list enforcement** ✅ done — `migrations/0032_sms_identities.sql`, `crates/buzz-db/src/sms.rs`. Not yet applied against a live Postgres (no infra spun up this pass).
7. **Twilio signature validation + inbound webhook route** ✅ done — `crates/buzz-relay/src/twilio_auth.rs` (HMAC-SHA1 validation, test vectors independently cross-checked via `openssl` and Python outside this codebase), `crates/buzz-relay/src/api/sms.rs` wired at `POST /hooks/sms/inbound`. Verified: unit tests pass, `cargo clippy -D warnings` clean.
7b. **Event synthesis** ✅ done — an allowed message now produces a real `KIND_STREAM_MESSAGE` (kind 9, **not** `KIND_STREAM_MESSAGE_V2` as first guessed — corrected after checking that buzz-acp's default Mentions-mode subscribe filter, `resolve_channel_filters` in `buzz-acp/src/config.rs`, only wakes agents on kind 9; a V2-only event would never trigger the persona meant to read it) tagged `h`/`sms_from`/`sms_sid`/`p`/`project`, signed by the relay's own keypair, mirroring `workflow_sink.rs`'s proven pattern. New `twilio_sms_inbox_channel` config (single global inbox channel, v1 simplification). Verified: `build_tags()` is a pure, unit-tested function (6 tests). **Not verified:** the actual DB insert (`insert_event_with_thread_metadata`) against a live Postgres — no DB infra available this session.
8. **Operator persona — fast path (default_project set)** ✅ done (2026-08-14, commit `60d59f9`) — `crates/buzz-persona/packs/sms-operator/`. The per-message routing half of this slice (b) was already covered by `resolve_turn_routing`/`batch_project_tag` from the earlier session-caching fix; what remained was (a), the actual persona file. Confirmed `format_event_block` in `queue.rs` already renders an event's raw tags verbatim into the `[Buzz event]` block, so the persona can read the `project` tag straight out of its own turn context with no new plumbing. `buzz pack validate`/`inspect` clean; a real `buzz-acp --pack crates/buzz-persona/packs/sms-operator` run logs `loaded persona pack pack=com.buzz.sms-operator persona=sms-operator` before failing later on an unrelated missing `goose` binary on this machine. **Not yet verified:** an actual inbound-SMS event dispatching this persona end-to-end (needs a live relay).
9. **Operator persona — ambiguous path** ⚠ partially done (2026-08-14) — the persona's prompt (same pack as slice 8) already instructs it to detect an absent/unrecognized `project` tag and reply with a disambiguation prompt instead of doing any work, and to *not* pretend a follow-up "1"/"bidcraft" reply can re-route it. What's still missing, and is a real gap, not a formality: there is no mechanism anywhere in this codebase for a sender's reply to actually update `sms_identities.default_project` — no `buzz-cli`/`buzz-admin` surface touches that column today. Until that exists, disambiguation is one-directional: the persona can ask, but nothing closes the loop. Verify (unchanged): seed a row with `default_project = NULL`, post an inbound event, confirm the persona posts a disambiguation reply instead of dispatching.
10. **Outbound SMS sink** ✅ code done (2026-08-14, commit `04bf428`) — `crates/buzz-relay/src/sms_sink.rs`, hooked into `dispatch_persistent_event_inner` (the same post-persist seam workflow triggering uses), spawned rather than awaited so Twilio can never stall dispatch. Destination resolution is deliberately narrow: the event must be in the inbox channel, must *not* itself carry `sms_from` (so an inbound event is never echoed back), and its `e` tag must resolve to a stored event that does — that tag is the only source of the destination number. One hop only; it does not walk a thread. New config: `TWILIO_ACCOUNT_SID`, `TWILIO_FROM_NUMBER`, and `TWILIO_API_BASE_URL` (the last exists so tests can target a local mock). Verified by 8 tests, 3 driving a real HTTP round-trip against a local mock listener asserting the actual Twilio wire format, plus non-2xx and unreachable-host failure paths. **Still needs the original live verification:** no send against real Twilio credentials has been made — that's part of slice 11 and needs a provisioned account/number.
11. **End-to-end SMS → project dispatch → reply** → verify: full loop — real inbound SMS from an allow-listed number with a clear `default_project`, agent dispatches and completes, outbound reply SMS arrives back at the sending number.
12. **Fetch-before-dispatch freshness check** ✅ done (2026-08-14, commit `563b85a`) — `commits_behind_upstream`/`warn_if_checkout_behind_upstream` in `pool.rs`, called once per new session creation (not per turn) right where `effective_cwd` is resolved. Deliberately warns rather than fast-forwards: only runs `git fetch` + `git rev-list --count HEAD..@{u}`, never touches the working tree or checked-out branch, since the checkout may have uncommitted work or be in use by another agent concurrently. Verified against real temp git repos (init/clone/push/fetch, not mocks): not-a-repo and no-remote both resolve to "can't tell," a fresh clone reports 0 behind, two commits pushed after cloning are correctly counted as 2 behind.

## ✅ RESOLVED 2026-08-13 (commit `2e745cf`) — both blockers below are FIXED

**Read this first: the two findings recorded below were true when written and are now
addressed.** They are kept for the reasoning, not as current state.

- **Finding 1 (packs inert) — FIXED.** `buzz-acp` now resolves a pack at startup via
  `--pack`/`--persona`, mapping `ResolvedPersona` onto `Config` (system prompt, model,
  title, runtime, instructions, env, subscribe). Explicit CLI/env values beat pack values
  (clap `ArgMatches` value_source), so a pack supplies defaults only. The `#`-stripping
  for subscribe names that the spec promised is implemented. A bad pack path is a hard
  startup error, never a silent no-op. **Covered by 29 passing
  `config::persona_pack_tests`** — including `pack_supplies_system_prompt_model_title_and_instructions`
  (proves a pack on disk really changes Config, not merely that it parses),
  `explicit_system_prompt_and_model_override_pack`, and
  `missing_pack_directory_is_a_hard_startup_error`.
- **Finding 2 (session caching) — FIXED.** `resolve_turn_routing()` + `batch_project_tag()`
  plus `session_cwds` on `SessionState`; invalidation fires **only when the resolved cwd
  actually changes**, ordered before the core-memory and canvas/title blocks so a
  replacement session rebuilds them. **Covered by 6 passing `pool::tests::routing_*` and
  `second_message_with_a_different_project_tag_creates_a_session_in_the_new_cwd`** — a
  three-turn (alpha→beta→beta) test capturing real ACP wire traffic that asserts exactly
  two `session/new` calls with the second in the new cwd on a different session id. That
  test **fails** if routing regresses to first-message-only, which was the whole trap.

**Security note (new surface):** the project id now arrives from an event tag and is
therefore attacker-influencable. It is only ever a lookup key into the operator-configured
`--project-paths` map, never joined into a path; unknown ids fall back to the channel
binding. `routing_hostile_project_ids_cannot_escape_the_configured_map` drives 17 hostile
ids (traversal, absolute, UNC, null byte, `$HOME`, `%USERPROFILE%`, case/whitespace
variants) and also asserts a legitimate id still resolves, so it is not vacuously
rejecting everything.

**Still NOT verified (as of the routing/pack work above):** no live relay, harness, or
agent process had been run at that point. All of it was unit-level verification (re-run
directly, not taken from build-agent reports). See the live-deployment section at the
bottom for what has since been verified against a running relay.

**Build environment gotcha:** the `C:\` drive on this machine is 100% full, which breaks
the MSVC linker (`LNK1108: cannot write file at 0x0`). Prefix cargo with
`TMP=E:/tmp-build TEMP=E:/tmp-build` to build at all.

## 2026-08-14 correction: `cargo test -p buzz-acp` was not fully green

Earlier passages above cite specific passing test counts (persona_pack_tests,
routing_*, build_tags) — those numbers were re-verified and are accurate. But
nothing here previously disclosed that **the full crate was not clean**: a
branch-wide audit found `cargo test -p buzz-acp` at 810 passed / 24 failed.
4 of those failures were a real bug in this crate's own tests (commit
`18bd342`): four `pool.rs` lifecycle tests spawned `"bash"` for their
fake-ACP-agent script, which resolves to WSL bash on this machine — WSL
mounts drives at `/mnt/e/`, not `E:/`, so a Windows temp capture path was
neither a valid WSL path nor writable as given. Fixed by switching those
tests to `"sh"` (matching the already-correct pattern in
`spawn_fake_session_agent`) and normalizing backslashes before shell-quoting.
Full suite is now 823 passed / 11 failed; the remaining 11 are unrelated
pre-existing `acp.rs` steer/timing races, not part of this feature.
Stray on-disk artifacts this bug produced (`st.txt` + 9 malformed filenames)
were deleted (commit `8c502c1`), with a `.gitignore` safety net added.

**Lesson carried forward:** "tests pass" claims in this doc should state
which tests, not imply the whole crate — re-verify against a fresh
`cargo test -p buzz-acp` count before trusting prior green claims here.

## ⚠ Two blocking findings (verified 2026-08-13) — these invalidated the original slice-8 plan (NOW FIXED, see above)

Both were found by an adversarial research pass and then **independently re-verified against the
real code** before being recorded here. They change what "add this as a plugin" can mean today.

### Finding 1: Persona Packs are inert at runtime — `buzz-acp` never loads one

The pack *format* is real, well-specified (`PERSONA_PACK_SPEC.md`), and has a working
parser/validator in `buzz-persona` + `buzz pack validate`/`inspect` in the CLI. But **nothing
loads a pack at agent-run time.**

Verified directly, not taken on report:
- `grep -rn "buzz_persona" crates/buzz-acp/src crates/buzz-acp/tests` → **zero hits**, despite
  `crates/buzz-acp/Cargo.toml:22` declaring `buzz-persona = { path = "../buzz-persona" }`. It is
  a dead Cargo edge.
- buzz-acp has **no** `--pack` / `--persona` flag and no `BUZZ_ACP_PERSONA_*` env var.
- `Config::persona_env_vars`'s doc comment claims it is "Populated from persona pack resolution" —
  **that comment is false**; the only code that pushes into it is the Codex sandbox network var.
- A persona's `subscribe:` field survives parsing into `ResolvedPersona.subscribe`, but its only
  consumer in the whole repo is the `buzz pack inspect` printout — the documented mapping to
  `Config.subscribe_mode`/`channels_override` **is never performed**.
- Spec §11's distribution surface (`buzz pack ... --output`, `buzz install`, `pack.lock`,
  `.buzzpack.sha256`, `~/.buzz/packs/`) does not exist; only `validate` and `inspect` are wired.

**Consequence:** writing `sms-operator.persona.md` would produce a file that validates cleanly and
does nothing. `buzz pack validate` printing "Valid." means *it parses*, not *it will run*.
**Deploying an agent today** = create it in the desktop app (`personas.json`), or set
`BUZZ_ACP_SYSTEM_PROMPT` / `BUZZ_ACP_MODEL` / env vars on the `buzz-acp` process directly.

**Consequence for the "plugin" question:** the operator can be *authored* as a pack for future
portability, but a pack is **not a deployment mechanism** today. Making it one is its own project
(wire pack resolution into buzz-acp startup), not a step in this feature.

### Finding 2: Per-message project routing is blocked by session caching

Routing on a per-message `["project", …]` tag is *not* just "extend `resolve_effective_cwd`":
- The triggering event and its tags **are** in scope at the call site (`batch` is live and
  un-moved at `pool.rs:1684`), so reading the tag is genuinely a small change. That part is fine.
- **But** `pool.rs:1678` short-circuits to a **cached session** and never reaches the resolver, and
  a session's `cwd` is immutable after `session/new` (`acp.rs:653`).

**Consequence:** a naive implementation routes correctly for the *first* message in a channel and
then silently ignores the project tag forever after — and it **passes a single-message test**,
which is the worst possible failure shape. Doing this properly needs a `session_cwd` field on
`SessionState` (`pool.rs:108-132`) plus pre-emptive invalidation when the resolved cwd changes,
touching both invalidation methods (`pool.rs:151-168`) which are load-bearing for core memory,
canvas, delivery state, and turn counters.

**Status:** deliberately NOT implemented. Landing the tag-read alone would look green and be wrong.

## Open questions for Michael

- Should `linked_pubkey` be required for `allowed = true` rows, or is the shared "SMS relay" service pubkey acceptable for allow-listed-but-unlinked numbers?
- Confirm Buzz dispatch's first live test target: construct-pro's `map.md` WF-01 (open, unblocked, small), or a Buzz-specific ticket filed in that same queue instead?
- Do you already have a Twilio account and a number, or does that need to be set up before slice 6 can be tested against real signatures (vs. a stubbed/test AuthToken)?
- ~~Is `E:/Projects/buildbid` the real checkout for `mfethe1/bidcraft`~~ — resolved: it's `E:/Projects/buildbid/bidcraft-repo`.
- Which `.claude/worktrees/buzz-<slug>` name to use for the construct-pro live dispatch test, and whether to run slices 4/5/11 (live dispatch) as a supervised session rather than unattended.

---
Sources: buzz-external-integration-design workflow (5 research agents), run 2026-08-12
Last updated: 2026-08-12

## 2026-08-14: first live deployment — inbound path verified against a running relay

Deployed to Railway (project `resplendent-patience`) as a **separate, isolated** service
`buzz-relay-sms` with its own Postgres and Redis, at
`https://buzz-relay-sms-production.up.railway.app`. The pre-existing `block/buzz:main`
relay and its database were deliberately left untouched — see "why not shared" below.

### Verified live (not unit-level)

- **Migration `0032_sms_identities` applied against a real Postgres** — the relay logged
  `Database migrations complete` on boot. Previously this was the standing "not verified
  against a live Postgres" gap for slices 6/7b.
- **`POST /hooks/sms/inbound` exists and is reachable** — returns 403/415 rather than the
  404 the upstream image returns, which is the direct proof the route is live.
- **The endpoint is not an oracle.** A form-encoded POST with *no* `X-Twilio-Signature`
  and one with a *bogus* signature both return `403` with a byte-identical
  `{"error":"request not accepted"}` body (diffed, not eyeballed). This is the
  anti-probing property the design called for, now confirmed on a real deployment.
- **Fail-closed when unconfigured.** The above 403s are currently produced by the
  "Twilio not configured" guard (no `TWILIO_AUTH_TOKEN` set yet), which is itself the
  correct documented behavior — it rejects rather than skipping validation. **Note the
  precise limit of this evidence: it verifies fail-closed-when-unconfigured, NOT that
  signature validation accepts a genuine Twilio signature.** That needs real credentials
  and remains unverified.

### Still NOT verified

Real Twilio credentials, a genuinely Twilio-signed inbound request, allow-list acceptance
of a real number, the outbound `sms_sink` against Twilio's live API, the operator persona
dispatching, and any live external-repo dispatch (slices 4/5/11).

### Deployment gotchas worth keeping

1. **Railway's `redeploy` re-runs the same snapshot**, it does *not* rebuild from the
   configured branch. It silently redeployed the fork's stale `main` and every Docker
   layer came back cached — which looks exactly like a successful build of the intended
   code. A **push to the tracked branch** is what triggers a real build. Check
   `list-deployments`' `meta.commitHash`/`meta.branch` before trusting a green deploy.
2. **The git object-store conformance probe is a hard startup gate** and requires a
   working S3/MinIO backend; it aborts the relay with
   `git conformance probe failed: s3 backend error ... localhost:9000` on any deployment
   without one. Set `BUZZ_GIT_CONFORMANCE_PROBE=false` for SMS-only test relays. This is
   unrelated to SMS but will burn time if unexpected.
3. **Why not share the existing relay's Postgres:** the running relay's image
   (`ghcr.io/block/buzz:sha-cd2125c`) predates migrations `0029`/`0030`, which add
   write-fence *triggers to every community-scoped table* and **drop** unique/check/FK
   constraints. Pointing this branch at that database would have schema-migrated a live
   production DB underneath a binary that predates the change. Isolated infra avoided it.

## 2026-08-14: inbound path VERIFIED END-TO-END against the live relay

The full inbound chain now works on the deployed test relay, with real Twilio
credentials. This closes the "no live relay has been run" gap for Part B's
inbound half.

**What was proven, and how:**

1. **Twilio signature validation accepts a genuine signature.** A request signed
   with a real HMAC-SHA1 over `URL + sorted(key+value)` using the account's live
   auth token was accepted. Previously only the *rejection* path had been
   exercised.
2. **The allow-list actually gates.** The decisive evidence is a before/after
   pair on the *same signed request*: it returned `403 {"error":"request not
   accepted"}` while `sms_identities` had no row for the sender, and
   `200 {"status":"accepted"}` immediately after inserting one — the only
   variable changed being the DB row. That rules out "the endpoint accepts
   anything signed" and "the 403 was really a signature failure" in one shot.
3. **Event synthesis is correct.** Reading the inbox channel back through
   `buzz messages get` returns the synthesized event:

   ```json
   {"content":"Buzz SMS live test - summarize open work","kind":9,
    "pubkey":"b07c536c…(relay)",
    "tags":[["h","707fefeb-53ec-4e1e-8195-ace864cdbafe"],
            ["sms_from","+18657192069"],["sms_sid","SMlivetest0002"]]}
   ```

   Note `kind: 9` — the corrected choice recorded in slice 7b (a
   `KIND_STREAM_MESSAGE_V2` event would never wake a Mentions-mode agent).
   No `project` tag appears, which is *correct*: this sender's
   `default_project` is NULL, so this is the ambiguous case the operator
   persona is written to handle.

**Deployment shape:** service `buzz-relay-sms` on Railway at
`https://buzz-relay-sms-production.up.railway.app`, isolated Postgres/Redis,
private `sms-inbox` channel `707fefeb-53ec-4e1e-8195-ace864cdbafe`, Twilio
number `+13173155284` webhook pointed at `/hooks/sms/inbound`.

**Operational note for anyone reproducing this:** the allow-list row had to be
inserted with `railway ssh --service <pg> -- psql …`, because no CLI or API
surface writes `sms_identities` (the known slice-9 gap) and the service had no
reachable public Postgres endpoint. `railway ssh` re-joins its COMMAND argv
through a shell, so the SQL must carry its *own* inner quotes to survive.

### Still NOT verified

- **Outbound SMS.** The account's A2P 10DLC campaign is in `FAILED` state
  (error 30909, CTA/message-flow rejection), so replies from this 10DLC number
  to US handsets are likely to be carrier-filtered (error 30034) regardless of
  `sms_sink` correctness. `sms_sink` has therefore still never run against
  Twilio's live API.
- **Operator persona dispatch.** Requires a `buzz-acp` harness process
  subscribed to the inbox channel; none has been run.
- **External-repo dispatch** (slices 4/5/11) — unchanged, still deferred.
