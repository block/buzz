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

## Build Windows installers locally

Run these commands from native PowerShell, not through the checked-in Unix
wrappers under `bin/`. Install dependencies once, and again when
`pnpm-lock.yaml` changes:

```powershell
cd H:\path\to\buzz
pnpm install --frozen-lockfile
```

Build the five release sidecars required by
`src-tauri/tauri.windows.conf.json`, then copy them to the target-qualified
names Tauri expects:

```powershell
cargo build --release `
	-p buzz-acp `
	-p buzz-agent `
	-p buzz-dev-mcp `
	-p git-credential-nostr `
	-p buzz-cli

$target = "x86_64-pc-windows-msvc"
$destination = "desktop\src-tauri\binaries"
New-Item -ItemType Directory -Force $destination | Out-Null

"buzz-acp", "buzz-agent", "buzz-dev-mcp", "git-credential-nostr", "buzz" |
	ForEach-Object {
		Copy-Item -Force `
			"target\release\$_.exe" `
			"$destination\$_-$target.exe"
	}
```

Close any running `buzz-desktop.exe` before packaging because Windows locks the
release executable. Then build the application and its MSI and NSIS installers:

```powershell
pnpm -C desktop tauri build
```

`pnpm tauri build` runs the frontend build automatically. Do not run
`pnpm build` separately and do not add a temporary Tauri config. Installers are
written under `desktop\src-tauri\target\release\bundle\msi` and
`desktop\src-tauri\target\release\bundle\nsis`.

## Structure

- `src/shared` - reusable app-wide code (`ui`, `lib`, `styles`)
- `src/features` - feature modules (vertical slices)
- `src/app` - top-level app composition
