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
keep the existing connection flow; discovery failures show a retryable check
failure. Switching communities invalidates pending checks. The card does not
reset keys or remove saved communities.

Packagers may set both `BUZZ_BUILD_REQUIRED_AUTH_UPGRADE_RELAY` (an exact `ws` or
`wss` community URL) and `BUZZ_BUILD_REQUIRED_AUTH_UPGRADE_URL` (an HTTPS installer
URL). The optional destination is compiled into the native command and exposed
only for the matching community on Apple Silicon macOS. Unconfigured builds and
other platforms show administrator guidance with no download link. Discovery
never supplies a trusted download or sign-in origin.

Before release, exercise a native build with disposable identities: join a
required host, reopen its saved community, switch to an optional host, then
reconnect after its requirement changes. Verify timeout/503 recovery, Retry,
community navigation, the configured download action, and retained keys and
saved communities. Repeat in an unconfigured OSS build. Native HTTP tests and
browser tests cover different boundaries; neither substitutes for installed-app
and human acceptance. Refresh the pinned fixtures from the merged BUZZ-511
producer before readiness. Host-policy rollout and maintenance-feed promotion
are separate release operations.
