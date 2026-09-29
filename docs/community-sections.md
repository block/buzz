# Community sections

Admin-defined channel groupings that members **opt into**. Distinct from
personal custom sections (`kind:30078` / `d=channel-sections`).

## Product

1. Members turn on **Use Community Sections** in Settings (master toggle; default on).
2. Admins configure sections in **Settings → Community sections**.
3. Each section lists channel ids (same list for every member).
4. Every member sees the catalog and can **Subscribe** (toggle) while the master
   switch is on.
5. Subscribed sections appear on the left nav with a distinct community-sections
   icon; unsubscribed ones do not.
6. While the master switch is on, channels that belong to a subscribed community
   section are **locked** in that section (no drag / Move to section into
   personal sections). The catalog stays admin-owned.
7. When the master switch is off, subscribed sections hide from the left nav,
   subscribe switches disable, and those channels can move into personal
   sections again. Personal sections are never deleted (empty sections stay).

## Data model

### Catalog (community-scoped, admin-authored)

| Field | Value |
| --- | --- |
| Kind | `30625` (`KIND_COMMUNITY_SECTIONS`) |
| d-tag | `buzz:community-sections` |
| Auth | Owner/admin only (relay ingest) |
| Scope | Tenant-global NIP-33 replaceable |

```json
{
  "version": 1,
  "sections": [
    {
      "id": "uuid",
      "name": "Engineering",
      "icon": "optional",
      "order": 0,
      "channelIds": ["channel-id-1", "channel-id-2"]
    }
  ]
}
```

Local mirror: `buzz-community-sections.v1:<relayUrl>`.

### Subscriptions (per member, client-local for MVP)

| Field | Value |
| --- | --- |
| Storage | `buzz-community-section-subs.v1:<pubkey>:<relayUrl>` |
| Shape | `{ "version": 1, "subscribed": { "<sectionId>": true } }` |

### Master toggle (per member, client-local)

| Field | Value |
| --- | --- |
| Storage | `buzz-community-sections-enabled.v1:<pubkey>:<relayUrl>` |
| Values | `1` / `0` (default ON when unset) |

Cross-device sync (NIP-78 encrypted preference, similar to project sidebar
membership) is a deliberate follow-up — not in this MVP.

## UI

- **Settings → Communities → Community sections**: master **Use Community
  Sections** switch; catalog + subscribe switch for everyone; create/edit/delete
  for owner/admin.
- **Left nav**: subscribed community sections render via `SidebarSection` (with
  a `LayoutList` title icon) above personal custom sections. Those channel ids
  are excluded from the personal-section / Channels buckets while the master
  toggle is on and the section is subscribed.
- Desktop maps `restricted: unknown event kind` on publish to a clear deploy
  message pointing here (prod relays that predate 30625).

## Deploy (kind 30625 on the relay)

`buzz.huladesk.com` (and any other prod relay) must run a relay build that
includes `KIND_COMMUNITY_SECTIONS` in ingest (`required_scope_for_kind` +
`authorize_community_sections`). Without that, Add/Save fails with
`restricted: unknown event kind`.

### What's already in this branch

Relay + core patches ship with the community-sections commit on
`wip/huddle-and-community-sections` / PR #166:

- `crates/buzz-core/src/kind.rs` — `KIND_COMMUNITY_SECTIONS = 30625`
- `crates/buzz-core/src/community_sections.rs` — payload validation
- `crates/buzz-relay/src/handlers/ingest.rs` — UsersWrite + owner/admin authorize

### How JM deploys to `buzz.huladesk.com`

1. **Build a relay image from this branch** (or from `main` once the relay
   patch is merged), e.g. via the usual Buzz/Hula image pipeline, or locally:
   ```bash
   # from repo root — tag however you promote to huladesk
   docker build -f Dockerfile -t ghcr.io/<your-org>/buzz:community-sections-$(git rev-parse --short HEAD) .
   ```
2. **Point the huladesk relay at that image** and roll it (Compose example):
   ```bash
   cd deploy/compose
   # in .env:
   # BUZZ_IMAGE=ghcr.io/<your-org>/buzz:community-sections-<sha>
   ./run.sh restart   # or your huladesk k8s/Argo rollout
   ```
3. **Confirm ingest** after Ready: publishing a 30625 catalog from desktop
   Settings → Community sections → Add should succeed (no unknown-kind toast).
4. Optional staging path: `.github/workflows/staging-dev-relay-image.yml` +
   `docs/staging-dev-relay-images.md` if you want a pre-merge image before
   promoting to prod.

Desktop alone cannot fix prod; the relay binary must accept 30625.

## Files

- `crates/buzz-core/src/community_sections.rs` — payload validation
- `crates/buzz-core/src/kind.rs` — `KIND_COMMUNITY_SECTIONS = 30625`
- `crates/buzz-relay/src/handlers/ingest.rs` — authorize + UsersWrite
- `desktop/src/features/community-sections/**` — catalog, subs, master toggle, hooks, settings
- `desktop/src/features/sidebar/ui/AppSidebar.tsx` — nav rendering + lock
- `desktop/src/features/settings/ui/SettingsPanels.tsx` / `SettingsView.tsx`

## Next steps

1. Encrypt-and-sync subscriptions / master toggle across devices (30078 preference blob).
2. Optional default-subscribe for newly created sections.
3. Drag-reorder community sections in admin UI (order field already exists).
4. Hide channels the member cannot access from the section list.
5. E2E: admin publish → member subscribe → section appears in sidebar.
