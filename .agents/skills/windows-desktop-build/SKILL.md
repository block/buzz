---
name: windows-desktop-build
description: "Build or package the Buzz Tauri desktop app on Windows. Use when: asked to build the Windows desktop executable, MSI, NSIS installer, run pnpm tauri build on Windows, prepare Tauri externalBin sidecars, fix missing target-qualified sidecar errors, or diagnose a locked buzz-desktop.exe during packaging."
---

# Windows Desktop Build

Use the repository's human-maintained build procedure in
[`desktop/README.md`](../../../desktop/README.md#build-windows-installers-locally).

Key constraints:

- Run native `cargo`, `pnpm`, and PowerShell commands on Windows; do not invoke
  the Unix wrappers under `bin/`.
- Build and copy all five release sidecars before invoking Tauri.
- Stop a running `buzz-desktop.exe` before rebuilding to avoid Windows file
  locking.
- Use `pnpm -C desktop tauri build` for the final package. It runs the frontend
  build and emits MSI and NSIS installers; do not add a separate frontend build
  or temporary Tauri configuration.
