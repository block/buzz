# CONTEXT.md — Project Context & Handoff Document

## Architecture Overview
- **Core Technology Stack**:
  - Relay: Rust WebSocket relay server (`buzz-relay`, NIP-29 group scoping, NIP-42 auth).
  - Desktop: Tauri 2 + React 19 + TypeScript + Vite + Tailwind CSS (`desktop/`).
  - Native Backends: macOS (`objc2-user-notifications`), Linux (`notify-rust`), Windows (`tauri-winrt-notification`).
  - Storage: Postgres 17 (`buzz-db`), Redis 7 (`buzz-pubsub`), Typesense (`buzz-search`), OS Keyring (`system-keyring`).

## Deploy Command
- **CI / Canary Pipelines**:
  - `just ci` — full local gate (`fmt` + `clippy` + desktop lint + unit tests).
  - GitHub Actions Workflows:
    - `.github/workflows/ci.yml` — auto-runs on pushes to `main`.
    - `.github/workflows/windows-canary.yml` — Windows NSIS installer build.
    - `.github/workflows/signed-macos-canary.yml` — signed macOS DMG build.
    - `.github/workflows/linux-canary.yml` — Linux AppImage/DEB build.

## Manual Dashboard Config
- **Environment Variables & Secrets**:
  - `BUZZ_BUILD_GOOGLE_CLIENT_SECRET`: Google OAuth Desktop Client Secret (compile-time).
  - `BUZZ_BUILD_IDENTITY_PEPPER`: Pepper secret used for SHA256-based deterministic Nostr key derivation from Google `sub`.
  - `BUZZ_RELAY_URL`: Target default relay URL (e.g. `wss://k2alpha.communities.buzz.xyz`).
  - `BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY`: `"1"` to enable auto-connecting default relay on first launch.

## Knowledge Items (KIs)
- **Branching Topology**:
  - `main` branch: Kept aligned with official upstream `block/buzz` (`https://github.com/block/buzz.git`).
  - `google-sso` branch: Dedicated fork branch preserving Google SSO peppered identity derivation & direct relay connection work.
- **Native Windows Notifications**:
  - Webview2 `window.Notification` on Windows 10/11 Action Center fails silently without AUMID registration.
  - Native Windows Toast Notifications use `tauri-winrt-notification` (`Toast::new(&app_id)`) in `desktop/src-tauri/src/commands/notifications.rs` and `isWindowsPlatform()` in `desktop/src/features/notifications/lib/desktop.ts`.
  - `Toast::text1` and `Toast::text2` consume `self` by value; builder methods must reassign instances (`toast = toast.text2(...)`).
- **Cross-Platform Target Gating (`mouse_nav.rs`)**:
  - macOS-only AppKit/Block modules (`block2`, `objc2_app_kit`) must be gated with `#[cfg(target_os = "macos")]` and provide a `#[cfg(not(target_os = "macos"))]` no-op stub (`pub fn init`) to maintain clean cross-platform builds on Windows and Linux.
- **GitHub Actions Workflow Parameters**:
  - `actions/setup-node` input parameter is `cache`, not `package-manager-cache`. Invalid parameter keys trigger workflow step validation failures.

## Pending Tasks
- **Google SSO Onboarding Race Condition**:
  - `useCommunityInit.ts` auto-connects to default relay on initial mount before Google SSO runs when `identity.storage === "ephemeral"`.
  - Fix: Guard `useCommunityInit` to skip `initFirstCommunity` when `identity.storage === "ephemeral"`, allowing Google SSO to derive and import the true identity first.
