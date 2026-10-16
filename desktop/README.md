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

## Prepare a Windows development host

Buzz's checked-in `bin/` files are Unix Hermit wrappers and cannot bootstrap
the toolchain from Git Bash or PowerShell on Windows. Use native PowerShell and
Windows-native tools for desktop development and packaging.

### 1. Install the command-line toolchain

Open a Shell and install Git, Node.js, Rustup, and Just with WinGet:

```powershell
winget install --id Git.Git -e
winget install --id OpenJS.NodeJS.LTS -e
winget install --id Rustlang.Rustup -e
winget install --id Casey.Just -e
```

Node provides `node`, `npm`, and Corepack. Rustup manages the Rust compiler and
Cargo version selected by this repository. Just runs repository tasks. Restart
your shell or IDE after these installations so new terminals inherit the
updated user and system `PATH` values.

Windows CI currently uses Node 24.14.1. The repository pins pnpm 11.4.0 in the
root `package.json` and Rust 1.95.0 in `rust-toolchain.toml`.

### 2. Enable pnpm through Corepack

Node is normally installed under `C:\Program Files`, so creating Corepack's
`pnpm` and `pnpx` shims beside Node requires an elevated shell. Open
**PowerShell as Administrator** and run:

```powershell
corepack enable pnpm
```

`corepack enable` installs the launch shims; it does not necessarily download
pnpm itself. Close the elevated shell, open a normal PowerShell terminal in the
repository, and run the version checks below. The first `pnpm --version` call
reads the root `packageManager` field and downloads pnpm 11.4.0 into Corepack's
per-user cache if it is not already present.

```powershell
cd DRIVE:\path\to\buzz
node --version
pnpm --version
rustc --version
cargo --version
just --version
```

The first `rustc` or `cargo` invocation in this repository may similarly make
Rustup download the Rust 1.95.0 MSVC toolchain selected by
`rust-toolchain.toml`. Allow those first-run downloads to finish before
continuing.

If a newly opened Shell still cannot find Cargo, first restart the shell or the IDE.
As a temporary fix for the current shell only, prepend Rustup's normal binary directory:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
```

This command does not install Rust. If `cargo.exe` is missing from that
directory even though WinGet reports Rustup as installed, repair the stale
installation by reinstalling the official package:

```powershell
winget install --id Rustlang.Rustup -e --force
```

### 3. Install the native C++ build tools

Tauri and the Rust Windows target require Microsoft's linker, C/C++ libraries,
Windows SDK, CMake, and related build tools. Install Visual Studio 2022 Build
Tools from PowerShell:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
	--override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Alternatively, open **Visual Studio Installer**, select **Build Tools 2022**,
choose **Modify**, and install **Desktop development with C++**, including the
MSVC v143 build tools, a Windows 11 SDK, and C++ CMake tools for Windows.

Restart Windows after installing or changing the Visual Studio Build Tools.
Windows 11 normally includes the WebView2 runtime used by Tauri; install the
Microsoft Edge WebView2 Evergreen Runtime only if Tauri reports that it is
missing.

### 4. Install dependencies and run checks

Use a normal, non-elevated PowerShell terminal for repository work:

```powershell
pnpm install --frozen-lockfile
pnpm -C desktop typecheck
pnpm -C desktop test
pnpm -C desktop dev
```

The final command starts the frontend-only Vite development server.
Run `pnpm install --frozen-lockfile` again whenever `pnpm-lock.yaml` changes.

## Build Windows installers locally

Build the five release sidecars required by `src-tauri/tauri.windows.conf.json`,
then copy them to the target-qualified names Tauri expects:

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

The same sidecars support native desktop development. To run the Tauri app in
development mode after copying them, use:

```powershell
pnpm -C desktop tauri dev
```

Close any running `buzz-desktop.exe` before packaging because Windows locks the
release executable. Then build the application and its MSI and NSIS installers:

```powershell
pnpm -C desktop tauri build
```

`pnpm tauri build` runs the frontend build automatically. The Installers
will be created in `desktop\src-tauri\target\release\bundle\msi` and
`desktop\src-tauri\target\release\bundle\nsis`.

## Structure

- `src/shared` - reusable app-wide code (`ui`, `lib`, `styles`)
- `src/features` - feature modules (vertical slices)
- `src/app` - top-level app composition
