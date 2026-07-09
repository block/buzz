# CHECKPOINT — Relay Invite Links (end-to-end) — 2026-07-09

Branch: `astro-relay-invites` (worktree `REPOS/buzz-worktrees/astro-relay-invites`, based on origin/main e0f76b0e).
Owner: astro (agent). Requested by baxen in buzz-invites channel (thread root f32e88ac…).
Status: **recon complete, design settled, no code written yet.** This file is the resume point.

## Agreed design (baxen approved in thread)

Stateless signed invite tokens — **no relay_invites table** (table is a future increment for
single-use/revocation). Anyone with the link can self-onboard until expiry; default expiry short (72h).

Deliverables (baxen's list):
1. UI to create the link from the relay members settings card (admin/owner).
2. UI acceptance of links to onboard to a relay (deep link + AddWorkspaceDialog paste).
3. Backend checking of the claim event.
4. Website in the middle: `/invite/<code>` page with download option + "Open in Buzz".

## Token format (decided)

`code = base64url(payload) + "." + base64url(hmac_sha256(relay_secret, payload))`
payload JSON: `{"c": community_id, "r": "member", "e": expires_at_unix, "n": nonce}`
- Secret: derive from `state.relay_keypair` secret key (HMAC key = sha256(secret_key_bytes || "invite-v1")) so no new config. `relay_keypair` at `crates/buzz-relay/src/state.rs:295`, built in `main.rs:298`.
- hmac/sha2/base64/rand already deps of buzz-relay (Cargo.toml:57,66,67,69).
- Community-scoped: claim handler must check payload community == tenant.community() (multi-tenant conformance).

## Wire protocol (NIP-43 aligned)

- **Mint**: kind 28935 (ephemeral range — never stored), sent by admin/owner over WS or HTTP bridge.
  Relay validates sender role via `get_relay_member` (same pattern as relay_admin.rs 9030 handler,
  incl. ±120s created_at freshness), replies with the code. NOTE: WS OK-message can't carry a payload
  well → **decision: mint via HTTP bridge instead**: `POST /invite/mint` (NIP-98 auth, role-gated),
  returns `{code, url, expires_at}`. Simpler than stuffing a code into an OK message. Constants go in
  `crates/buzz-core/src/kind.rs` next to KIND_NIP43_* (line ~249) if we do event-based mint later.
- **Claim**: kind 28934, user-signed, `["claim", <code>]` tag. Ephemeral. MUST work pre-membership:
  - WS path: handled in `handlers/event.rs` BEFORE the membership gate (like AUTH). Sender = event.pubkey.
  - Also add HTTP: `POST /invite/claim` (NIP-98-signed event in body, no membership enforcement) —
    desktop can call it before opening WS. Probably ship HTTP-only first; WS optional.
  - Handler: verify HMAC + expiry + community match → `add_relay_member(community, pubkey, "member", Some("invite"))`
    (db fn at `crates/buzz-db/src/relay_members.rs:97`, idempotent) → publish_nip43_member_added +
    publish_nip43_membership_list (`handlers/side_effects.rs:2704-2820`).
  - Rate limit: per-IP/per-pubkey simple counter (moka cache on AppState) — claim is reachable unauthed.

## Relay integration points

- Router: add routes in `crates/buzz-relay/src/router.rs` api_router block (~line 60): `/invite/mint`, `/invite/claim`.
  New module `crates/buzz-relay/src/api/invites.rs`, registered in `api/mod.rs` (pub mod invites;).
- Auth helpers to reuse: `verify_bridge_auth` (bridge.rs:28), `nip98_expected_url` (bridge.rs:161),
  `check_nip98_replay` (bridge.rs:102), tenant binding via `crate::tenant::bind_community` (see submit_event bridge.rs:557).
- Mint authz: `state.db.get_relay_member(tenant.community(), sender_hex)` role in (admin,owner) —
  mirror relay_admin.rs:133-142.
- Token helper: new `crates/buzz-relay/src/invite_token.rs` (mint/verify + unit tests) or inside api/invites.rs.
- On open relays (require_relay_membership=false): claim still inserts the member row (harmless, keeps roster).

## Web (`web/`)

- Routes: `web/src/app/routes.ts` (tanstack virtual-file-routes) — add `route("/invite/$code", "invite.$code.tsx")`.
- Page: install/download links + "Open in Buzz" button →
  `buzz://join?relay=<encodeURIComponent(relayWsUrl())>&code=<code>`.
  Reuse pattern from `web/src/features/repos/ui/ConnectButton.tsx`; `relayWsUrl()` in `web/src/shared/lib/relay-url.ts`.
- SPA fallback already serves unknown paths when BUZZ_WEB_DIR set (`router.rs:89-122`) — no server change needed.
- Download links: point at GitHub releases (check what marketing/download URL exists; placeholder ok).

## Desktop (`desktop/`)

- Deep link: extend `desktop/src-tauri/src/deep_link.rs` with `Some("join")` arm →
  emit `deep-link-join` with `{relayUrl, code}` (validate ws/wss like connect arm).
- Frontend listener: `desktop/src/shared/deep-link.ts` — new `listenForDeepLinks` case or separate listener:
  - Stash pending invite code (module-level or localStorage `buzz-pending-invite`),
  - addWorkspace + switchWorkspace (same as connect),
  - After workspace applied & identity exists → claim: POST /invite/claim with NIP-98 signed by
    identity (via `signRelayEvent` from `@/shared/api/tauri` + `getRelayHttpUrl()`, pattern:
    `desktop/src/shared/api/moderation.ts:222-243`).
  - Claim call site: onboarding membership check (`OnboardingFlow.tsx` checkMembershipDenied, line ~60)
    — if pending invite code exists, claim BEFORE checking membership; also in MembershipDenied add
    "Have an invite code?" paste field.
- First-run: `App.tsx:346` registers listenForDeepLinks only after WelcomeSetup completes? — VERIFY:
  deep link on cold start comes through as launch arg (`src-tauri/src/lib.rs:77-84`). Need pending-code
  persistence across the WelcomeSetup path: if `deep-link-join` arrives while needsSetup, seed
  WelcomeSetup relay URL + store code. Check how deep-link-connect behaves on first run today.
- Create-link UI: `desktop/src/features/relay-members/ui/RelayMembersSettingsCard.tsx` — add
  "Create invite link" button (visible to admin/owner, uses `useMyRelayMembershipQuery`), calls
  POST /invite/mint, shows copyable `https://<relay-http-host>/invite/<code>` + expiry note.
  New api fn in `desktop/src/shared/api/relayMembers.ts`.
- AddWorkspaceDialog (`desktop/src/features/workspaces/ui/AddWorkspaceDialog.tsx`): accept a full
  invite link OR code pasted into a new field; parse relay URL from link; claim after connect.

## Test plan

- Rust: unit tests for token mint/verify (expiry, tamper, wrong community); handler tests mirroring
  existing relay_admin tests; `cargo test -p buzz-relay invite`.
- Desktop: mjs unit test for parsing invite links (pattern: `messageLink.test.mjs`).
- Manual: two-instance flow vs local relay (see TESTING.md / LOCAL_RELAY_MEMBERSHIP_TESTING_RESEARCH.md in nest).

## Nest references

- `RESEARCH/RELAY_INVITE_FLOW_2026_07_09.md` — full investigation writeup.
- Thread: channel buzz-invites 68bd2cb8…, root f32e88ac….

## Remaining TODO (ordered)

1. relay: invite_token helper + tests
2. relay: api/invites.rs mint+claim routes + router wiring + tests
3. web: /invite/$code page
4. desktop: deep_link.rs join arm + TS listener + pending-code store
5. desktop: claim call in onboarding + MembershipDenied paste field + AddWorkspaceDialog field
6. desktop: mint UI in RelayMembersSettingsCard
7. e2e sanity + PR
