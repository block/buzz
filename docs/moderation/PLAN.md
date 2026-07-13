# Community-Admin Moderation Plan

**Goal (Tyler, 2026-07-01):** let community admins moderate their own communities.
Co-authored by Eva + Wren. Grounded against `buzz-1321-review @ 86d6388` and
`RESEARCH/NOSTR_CONTENT_REPORTING_MODERATION.md`. Re-verified against fresh
`REPOS/buzz-moderation-ref` @ main `1f5ba5b` (2026-07-07).

## 0. DECISIONS LOCKED (Tyler, 2026-07-07, #buzz-moderation thread e2a91af6)

All open calls below are now decided. The UX was refined by Eva + Wren to a
co-signed 9/10+ (Wren's line-cited corner-check: event `295db04e`; final
presentation: event `4273365f`).

1. **Roles: owner + admin only. No Moderator tier for now.** (Tyler, event `559f838f`.)
2. **Reporter identities visible to mods** in the queue (never to the reported author).
3. **Relay-key resolution DM** (Tyler, event `34de7dac`): on report resolution the
   relay key authors a regular nostr message stored in the DB. Container (Wren-verified):
   relay/moderation key creates/reuses a two-party DM channel `{mod key, user}` via the
   participant-hash-idempotent DM model (`buzz-db/src/dm.rs`), emits 39000 discovery
   (`hidden`, `t=dm`, `p` tags), inserts relay-signed kind-9 with `h=<dm_channel_id>`.
   One DM thread per user per community; relay publishes kind-0 "{Community} Moderation"
   identity. `moderation_reports` persists `reporter_pubkey`. Same primitive carries
   actioned-author notices and timeout/ban issuance. Non-replyable v1; replies (v2)
   route to mod queue as appeals.
4. **Ban/timeout gate at the auth seam** (Tyler, event `34de7dac`): evaluated after
   NIP-42 verify, before pubkey allowlist / `enforce_relay_membership`
   (`handlers/auth.rs` — gate goes immediately after ~L91). Banned ⇒
   `OK false "blocked: you are banned from this community"` + immediate WS close, zero
   further processing (`blocked:` is the NIP-01 standardized prefix; there is no
   `banned:` prefix; AUTH must be answered with `OK` per NIP-42). Live ban kill:
   cluster-wide disconnect-by-pubkey over Redis pub-sub; `CLOSED "blocked: ..."` per
   active sub, then socket close (needs a close-by-pubkey API on `ConnectionManager`).
   NIP-OA: gate checks the authenticating pubkey always + owner pubkey when present —
   owner ban cascades to agents; agent ban is agent-only; audit records matched
   principal, client never learns which. Timeout is a write-block, not a connection
   block: `OK false "restricted: you are timed out until <ts>"`, desktop disables
   composer with countdown chip. No silent write-drops.

## 1. Where this sits — two moderation layers, not one

Discord's published model (safety/our-approach-to-content-moderation) is explicitly
**two-layer**, and Buzz should mirror it:

| Layer | Discord | Buzz | Owner | This plan |
|-------|---------|------|-------|-----------|
| **Platform safety** | Trust & Safety team: CSAM image-hashing → NCMEC, ML network detection, human investigations | Relay operator: sha-match in S3, NIP-86 ban/takedown, NIP-62 vanish | Block / relay operator | **Adjacent** — Fizz's `MODERATION_SAFETY_SKETCH.md` owns it. We cite, don't duplicate. |
| **Community moderation** | Server owners + volunteer mods: AutoMod keyword/spam filters, Warning System | Community admins/mods: NIP-29 admin actions, report queue, in-community bans | Community admins | **THIS PLAN.** |

The severe-safety class (CSAM) is never delegated to community admins — it's a
platform-level hard-removal + legal-report path. Community moderation is the
subjective, per-community rule enforcement layer on top.

## 2. What PR #1321 already gives us (the foundation)

#1321 is **not** a moderation PR — it's the multi-tenant substrate that makes
per-community moderation *safe*:

- Every scoped row carries a **server-resolved `community_id`** (never caller-supplied).
- `TenantContext` is minted only on the host-resolution path and threaded through
  every scoped DB read + Redis publish — a proven cross-community isolation fence.

**Implication:** community-admin moderation authority is naturally tenant-scoped, but
only if target resolution also stays inside the fence. An admin of community A can
never reach community B's content when the query lists/deletes/bans with
`for_community(A)`. The dangerous edge is a new moderation signal whose *target* is a
bare event id, address, pubkey, or blob hash. Therefore every new moderation table gets
`community_id`, every new action takes `&TenantContext`, and every report/label target
is resolved under `tenant.community()` before it enters a queue or action row. No new
isolation primitive needed; reuse #1321's — but do not add any global target lookup in
the report/label path.

## 3. What already exists (build-on, don't rebuild)

From `crates/buzz-relay/src/handlers/side_effects.rs` + `buzz-core`:

- **NIP-29 admin kinds**, authorized in `validate_admin_event`:
  - `9000` put-user (add / assign role), `9001` remove-user (kick),
    `9002` edit-metadata, `9005` delete-event, `9008` delete-group,
    `9021`/`9022` join/leave request.
- **Role model** (`buzz-core/src/channel.rs`): `Owner > Admin > Member > Guest`, plus
  `Bot`. `is_elevated()` = Owner|Admin gates elevated-role grants. Agent-owner
  delegation lets the owning human act.
- **39000-series** group-state mirrors (metadata/admins/members/roles).
- **NIP-51 mute list** (kind:10000) constant exists — user-level, client-advisory.

## 4. The gaps (what this plan adds)

### Gap A — No distinct Moderator role
Today the only elevated tier is `Admin` (can manage members + settings). Discord's
model leans on a **volunteer-moderator** tier that can act on content/users but
*cannot* reconfigure the community or manage other mods.

**Recommendation after corner-check:** ship v1 **Admin-only for community-wide
moderation**, with an explicit capability helper, and defer a distinct `Moderator`
role until product needs delegated volunteer moderation.

Why not add `Moderator` in v1:

- Community-wide authority lives in the tenant-scoped `relay_members` table, not in
  channel `MemberRole`. Adding `Moderator` to `MemberRole` would migrate channel-local
  DB enum/API/UI/projection surfaces without actually creating tenant-wide authority.
- Adding `moderator` to `relay_members.role` is possible, but still touches admin
  commands, membership announcements, UI, downgrade paths, and tests. That is extra
  surface before we have the basic report/delete/ban loop working.
- NIP-29 interop is fine either way because roles are relay-defined/arbitrary, but
  clients that only reconstruct channel `39001` admins will not understand a
  community-wide moderator unless the relay explains/enforces it. A relay-signed
  tombstone is clearer than silently adding every community admin/mod to every group.
- A capability helper now makes v2 cheap: later `moderator` is a role-to-capability
  mapping, not an authorization rewrite.

V1 capability grid:

| Capability | Community owner | Community admin | Channel owner/admin | Member |
|------------|:---:|:---:|:---:|:---:|
| Delete any message in community (9005) | ✅ | ✅ | channel only | own only |
| Remove/kick user (9001) | ✅ | ✅ | channel only | self |
| Ban user from community | ✅ | ✅ | ❌ | ❌ |
| Timeout/mute user in-community | ✅ | ✅ | ❌ | ❌ |
| Resolve reports (queue actions) | ✅ | ✅ | optional channel-view | ❌ |
| Assign/revoke community Admin | ✅ | ❌ | ❌ | ❌ |
| Edit community-level settings | ✅ | ✅ | ❌ | ❌ |
| Delete community / hard-delete channel (9008) | ✅ | ❌ | channel owner only where already allowed | ❌ |

Open call for Tyler: **is a distinct Moderator tier required in v1?** My reviewed lean
is **no**: ship Admin-only, but structure authorization around capabilities so adding
`moderator` later is a narrow extension. If Tyler wants Discord-like volunteer mods on
day one, add `moderator` first to `relay_members.role` (tenant-level), advertise it in
`39003`, and only then decide whether channel `MemberRole` also needs a `Moderator`.

### Gap B — No NIP-56 reports (the report button)
**The single biggest missing primitive.** kind:1984 is referenced only as a CLI
filter example; there is no ingest, queue, or UI. Without it, mods have nothing to
act *on* — they can only delete what they personally see.

Pipeline (all tenant-scoped via #1321's fence; target resolution is the sharp edge):

```
client "Report" → kind:1984 event (p + e/x tags, report-type, free-text)
       │  ingest: validate_report_event (new, in side_effects.rs)
       ▼
  moderation_reports table (community_id, target_event/pubkey/blob,
       reporter, type, reason, created_at, status=Open)
       │  aggregate by target; weight trusted reporters (mods > members)
       ▼
  per-community moderation queue (buzz-cli / desktop admin surface)
       │  mod resolves: dismiss | delete (9005) | remove (9001) |
       │                ban | mute | escalate-to-platform
       ▼
  moderation_actions table (audit) + relay-signed tombstone (see Gap D)
```

Target-resolution rules to keep the tenant fence closed:

- `e` target: look up the event only with `tenant.community()`; infer `channel_id`
  from that row. If missing in this tenant, reject or store as an unresolved report —
  never search other tenants by event id.
- `x` blob target: resolve through tenant-scoped media references, e.g.
  `(community_id, sha256)` or `(community_id, event_id, sha256)`. A bare SHA-256 can
  be shared across tenants and must not grant cross-tenant visibility/action.
- `p`-only target: treat as a community-local report about that pubkey in the current
  tenant. It cannot imply a platform/global ban.
- NIP-32 labels (`1985`) in v2 follow the same rule: labels are advisory inputs until
  their target is resolved under the current `TenantContext`.

Key policy (from NIP-56 + the RESEARCH doc): **relays should not auto-moderate on
random-user reports** — they're gameable. Reports are a triage inbox, not an
auto-takedown trigger. Exception: trusted-admin reports of CSAM/illegal class can
fast-path to platform quarantine (Fizz's layer), not to community-admin discretion.

### Gap C — No persistent ban / in-community mute
`9001` remove-user is a kick — nothing stops re-join on an open channel. Add:
- `community_bans` table (`community_id`, pubkey, actor, reason, expires_at NULL=perm).
- Enforce at join (`9021`/`9000` self-add) and at event ingest for the community.
- **Mute/timeout** = time-boxed write-block (can read, cannot post) — a softer tool
  than ban, matches Discord's timeout. Same table, `muted_until` column.

### Gap D — No "Warning System" (why-was-this-removed)
Discord tells users *why* content was actioned. Buzz already has the right primitive:
`handle_delete_event_side_effect` soft-deletes the target, then emits a relay-signed
kind `40099` system message with `type: "message_deleted"`, `actor`, and
`target_event_id`. Extend that rather than inventing a new 48000-range event for v1.

Recommended shape:

- **Public/in-context tombstone:** relay-signed kind `40099`, rendered as "Removed by a
  community moderator" without exposing removed content. Add safe fields:
  `action_id`, `target_event_id`, `target_author`, `reason_code`, optional sanitized
  `public_reason`, and maybe `actor` if moderator identity is intentionally visible.
- **Private author notice:** separate p-gated/DM notification to the actioned user with
  the reason, community rule, and appeal/restore path. This is the closer Discord
  Warning System analog.
- **Internal action/audit row:** full moderator, report ids, reporter identities,
  evidence, and unsafe details stay admin-only.
- **NIP-32 label:** useful in v2 as an advisory/scanner signal, not the authoritative
  enforcement record. Labels can be consumed by clients; tombstones explain relay
  actions.

## 5. Standards to adopt vs. skip (from RESEARCH doc, cited to nostr-protocol/nips)

| NIP | Adopt? | Role in this plan |
|-----|--------|-------------------|
| **NIP-29** groups (9000–9022) | **Yes — already have it.** Extend authorization for community Admin + ban. | Relay-enforced community roles + admin actions. |
| **NIP-56** reports (1984) | **Yes — new.** Gap B. | User report button; interop-standard so other clients work. |
| **NIP-32** labels (1985) | **v2.** | Distributed/automated moderation output; feeds queue as a signal. Client-advisory blur/hide. |
| **NIP-51** mute (10000) | **Yes (exists).** | *User-level* self-mute — client-advisory, orthogonal to community moderation. Keep distinct from Gap C's community mute. |
| **NIP-86** relay-admin API | **Platform layer** (Fizz). | Operator ban/takedown at event/pubkey/IP grain. Not community-admin scope. |
| **NIP-62** vanish (kind:62) | **Platform layer.** | GDPR right-to-erasure; MUST fully delete + block rebroadcast. Legal, not community. |
| **NIP-72** moderated communities | **Skip.** | Deprecated upstream in favor of NIP-29 (README marks it unrecommended). |
| **NIP-09** deletion (kind:5) | Keep (exists). | Author self-delete only; advisory. Not a moderation tool for others' content. |

## 6. Phased rollout

**Phase 1 — community-admin MVP (this plan's core):**
1. Add a community moderation capability helper and extend `validate_admin_event` for
   community `owner|admin` authority (Admin-only v1 unless Tyler explicitly wants
   `moderator`).
2. NIP-56 kind:1984 ingest → tenant-scoped `moderation_reports`, with target
   resolution under `TenantContext`.
3. `moderation_actions` audit table + extended relay-signed kind `40099` tombstone for removals.
4. `community_bans` (+ mute/timeout column); enforce at join + ingest.
5. `buzz-cli moderation` queue commands (list/resolve/ban/mute) for admins.

**Phase 2 — UX + trust weighting:**
6. Desktop/mobile Report button + moderation queue surface.
7. Trusted-reporter weighting; report aggregation by target.
8. Warning-System notifications to actioned users.

**Phase 3 — distributed signals:**
9. NIP-32 labels (1985) as a queue input + client-advisory hide.
10. Cross-community shared blocklists (opt-in), platform-layer coordination.

## 7. Open calls for Tyler — ALL DECIDED 2026-07-07, see §0
- **Moderator role in v1, or Admin-only first?** (§4 Gap A — reviewed lean: Admin-only v1 with capability seams.)
- **Ban grain:** per-community only, or should a community ban be able to request a
  platform-level ban for severe cases? (Escalation path to Fizz's layer.)
- **Report visibility:** are reports mod-only, or does a reporter see resolution?
- **Warning System shape:** reviewed lean is relay-signed kind `40099` tombstone + private author notice; NIP-32 label is v2/advisory (§4 Gap D).

## Sources
- `buzz-1321-review @ 86d6388`: `crates/buzz-core/src/kind.rs`,
  `crates/buzz-core/src/channel.rs`, `crates/buzz-relay/src/handlers/side_effects.rs`.
- PR #1321 description (multi-tenant `community_id` / `TenantContext`).
- `RESEARCH/NOSTR_CONTENT_REPORTING_MODERATION.md` (cited to nostr-protocol/nips).
- Discord: safety/our-approach-to-content-moderation (2024-03-15).
- Fizz's `docs/MODERATION_SAFETY_SKETCH.md` (platform media-safety layer, adjacent).
- Wren corner-check: `PLANS/COMMUNITY_MODERATION_CORNER_CHECK_WREN.md`.
