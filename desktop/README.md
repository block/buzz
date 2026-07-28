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

## Local macOS Install

Local unsigned Tauri builds are ad-hoc signed, so macOS Keychain trusts the
exact binary hash. After each rebuild, the `buzz-desktop` Keychain item can ask
again even if "Always Allow" was selected before.

Use the local installer to sign Buzz with a stable local code-signing identity:

```bash
just desktop-install-local-macos --create-identity
just desktop-signing-status
```

After the first run, omit `--create-identity`. If macOS prompts for Keychain
access once after the signed install, choose "Always Allow"; later local rebuilds
signed with the same identity should keep the same Keychain authorization.

See [Desktop Local macOS Signing](../docs/desktop-local-macos-signing.md) for
the migration and new-machine runbook.

## Structure

- `src/shared` - reusable app-wide code (`ui`, `lib`, `styles`)
- `src/features` - feature modules (vertical slices)
- `src/app` - top-level app composition
