# Buzz

> [!IMPORTANT]
> **This is the deprecated Buzz 0.X desktop app.** Buzz 1.0 lives in
> [block/buzz-app](https://github.com/block/buzz-app). Build new desktop
> features there. Change this directory only to fix a 0.X bug, or to keep it
> compiling after a shared crate change.

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

The upgrade card directs public users to the fixed `https://buzz.xyz` URL to
download an up-to-date Buzz client on every supported desktop platform and for
every required community.
If the browser cannot open, the card shows the public URL for manual navigation.
Discovery never supplies a trusted download or sign-in origin.
