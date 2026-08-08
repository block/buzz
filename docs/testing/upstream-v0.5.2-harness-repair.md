# Command Adviser v0.5.2 Harness Packaging Repair

**Date:** 1 August 2026  
**Branch:** `codex/upstream-v0.5.2-sync`  
**Draft PR:** [NavigatorRAN/buzz#14](https://github.com/NavigatorRAN/buzz/pull/14)

## Fault and root cause

The installed v0.5.2 application reported `ACP harness command 'buzz-acp' was
not found` for every managed adviser. The installed app and its local build
output contained zero-byte, non-executable Tauri placeholders for all six
workspace sidecars. Agent records still correctly selected `buzz-acp`.

The local `desktop-release-build` recipe created compile-time placeholders and
then invoked Tauri without compiling or bundling the real Rust sidecars. Code
signing accepted those files, and the previous package inspection validated
only the Apple-input helper.

## Repair

- `desktop-release-build` now compiles all sidecar packages for the selected
  target and runs the existing `scripts/bundle-sidecars.sh` path before Tauri.
- `scripts/verify-desktop-sidecars.sh` rejects missing, empty, and
  non-executable Unix sidecars.
- The verifier runs after staging and against the final macOS application.
- The focused shell regression is included in the repository `check` gate.

## Build and installation evidence

- Built application:
  `desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Command Adviser.app`
- Installed application: `/Applications/Command Adviser.app`
- Recoverable broken application:
  `/Applications/Command Adviser.before-harness-packaging-fix-20260801-220850.app`
- Version: `0.5.2`; display name: `Command Adviser`; architecture: ARM64.
- Strict deep code-signature and macOS entitlement verification passed before
  installation and again after installation.
- Installed `buzz-acp` SHA-256:
  `84d9fcdb4dddbb483f0c8b23187975197ac41c924121824a9ae64218c5fa7d4a`.
- All six installed sidecars are non-empty Mach-O ARM64 executables. Their
  observed sizes ranged from 1,338,720 to 19,418,400 bytes.
- The installed `buzz-acp --help` command exits successfully and identifies
  itself as the ACP harness bridging Buzz events to AI agents.

## Data preservation and live state

- Managed-agent records before and after installation: `22`.
- Managed-agent store SHA-256 before and after installation:
  `8ae75cc693983b927d3251445515cb6b8332d29eb9db07d552125e3ae5e148ae`.
- Relay events before and after installation: `326`.
- Relay health after installation: `ok`.
- The installed desktop process launched successfully as PID `56005` during
  acceptance.

## Verification

- `scripts/tests/verify-desktop-sidecars-test.sh`: passed, including valid,
  missing, zero-byte, non-executable, and release-recipe cases.
- `cargo test --manifest-path desktop/src-tauri/Cargo.toml resolve_command --lib`:
  1 passed, 0 failed.
- Pre-push desktop native gate: 2,169 passed, 14 ignored; 3 audio
  diagnostics passed; branch-skew and native checks passed.
- Corrected `just desktop-release-build aarch64-apple-darwin`: passed and its
  post-package sidecar verifier passed.
- `git diff --check`: passed.
- Full `just ci`: Rust formatting and workspace Clippy passed, then the existing
  upstream-sync branch file-size ratchet stopped `desktop-check` in files not
  touched by this repair. The failure inventory includes oversized agent UI and
  managed-agent modules already present at the branch head; it is not a harness
  packaging regression.

The app-level agent Start action remains the final interactive acceptance
boundary. Automated macOS application control could not obtain the app's
accessibility state, so the test did not fabricate a start by rewriting agent
records or clearing their previous errors. Starting any adviser from the Agents
view should create a real `buzz-acp` process and replace the stale not-found
error with current harness output.
