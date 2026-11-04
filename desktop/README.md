# Buzz

Desktop chat shell with:

- Tauri + React + TypeScript + Vite
- Tailwind CSS
- shadcn/ui-ready shared components
- Biome (lint/format/check)
- Feature-driven frontend structure

## Scripts

- `pnpm dev` - run the web frontend
- `pnpm tauri dev` - run the desktop app
- `pnpm build` - typecheck and build frontend
- `pnpm typecheck` - TypeScript checks
- `pnpm lint` - Biome lint
- `pnpm format` - Biome format (write)
- `pnpm check` - Biome check

## Structure

- `src/shared` - reusable app-wide code (`ui`, `lib`, `styles`)
- `src/features` - feature modules (vertical slices)
- `src/app` - top-level app composition

### Required-community guidance in the legacy app

The legacy desktop reads public NIP-11 metadata before each WebSocket connection.
Only `requires_nip_fi: true` shows the upgrade card. Missing/false requirements
keep the existing connection flow. Discovery failures retain an unknown-policy
state and continue the existing socket/authentication path; the relay enforces
admission. Query/fragment URLs keep their existing socket behavior; the native
discovery reader rejects them before making an HTTP request. Credentials never
reach a fallback dial. Only explicit required policy opens the upgrade modal. Switching
communities invalidates pending checks. The card does not
reset keys or remove saved communities.

The upgrade card directs public users to `https://builderlab.xyz` to download
an up-to-date Buzz client. This default applies to every supported desktop
platform and required community, including unconfigured public release builds.
The card does not use the older private-host/platform build overrides. If the
browser cannot open, the card shows the public URL for manual navigation.
Discovery never supplies a trusted download or sign-in origin.

Before release, exercise a native build with disposable identities: join a
required host, reopen its saved community, switch to an optional host, then
reconnect after its requirement changes. Verify timeout/503 recovery, Retry,
community navigation, the public download action, and retained keys and
saved communities. Repeat in an unconfigured OSS build. Native HTTP tests and
browser tests cover different boundaries; neither substitutes for installed-app
and human acceptance. Refresh the pinned fixtures from the merged BUZZ-511
producer before readiness. Host-policy rollout and maintenance-feed promotion
are separate release operations.

Before any bb-public community is switched to required FI:

1. Choose and test a public `desktop-v0.5.x` maintenance candidate containing
   this patch. Review its main delta or validate a tag-based backport.
2. Publish its supported-platform installers through `.github/workflows/release.yml`,
   install and accept them, then use **Promote OSS Desktop Auto-Update** as
   documented in [RELEASING.md](../RELEASING.md#promote-an-oss-desktop-release-to-auto-update).
   Verify `buzz-desktop-latest/latest.json` lists that exact maintenance version
   and the expected macOS (both chips), Linux and Windows update artifacts.
   Publishing a tag alone does not update existing public clients.
3. Verify bb-public runs the merged BUZZ-511 producer with corrected effective
   `requires_nip_fi`: Off false, Shadow optional false, Shadow required true,
   Enforce true. Confirm live discovery and refresh the consumer fixtures.
4. Exercise the installed legacy maintenance client against the deployed relay,
   including the public builderlab.xyz action, before any required-host rollout.

The Block `mdx/sprout/latest.json` feed is a separate maintenance route and cannot
substitute for public updater delivery. Whether that route is needed remains an
owner decision. These are release prerequisites, not authorization to tag, build,
promote an updater feed, or change relay policy.
