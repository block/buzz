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
- **`devBuildLabel.ts` needs `git rm`**:
  - `desktop/src/shared/lib/devBuildLabel.ts` is dead code (superseded by the versioning scheme below) with zero remaining references, but couldn't be deleted from the environment that made this change — needs `git rm desktop/src/shared/lib/devBuildLabel.ts` as part of the next commit that touches this area.

## k2alpha Auto-Updater & Release Pipeline
- **Distribution model**: `.github/workflows/release.yml` builds, signs, and publishes a Windows NSIS installer + Tauri updater manifest to GitHub Releases on this fork (`github.com/ranjank2alpha/buzz`, public repo) whenever a `v*` tag is pushed. Existing installs poll `https://github.com/ranjank2alpha/buzz/releases/latest/download/latest.json` (configured in `desktop/src-tauri/tauri.conf.json`'s `plugins.updater.endpoints`) every 6 hours (`desktop/src/features/settings/hooks/use-updater.ts`).
- **Versioning scheme — load-bearing, do not deviate**:
  - The core app version stays `0.5.5` forever — it must never be bumped, since it tracks upstream Buzz's own version number.
  - k2alpha releases use a semver pre-release suffix instead: `0.5.5-4`, `0.5.5-5`, `0.5.5-6`, ... The number after the dash increments per k2alpha release and must stay purely numeric (Tauri's Windows/WiX bundler only accepts a numeric pre-release segment).
  - Bump this version string in exactly three places together: `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/Cargo.toml`, `desktop/package.json`.
  - Tag format is `v0.5.5-N` (e.g. `v0.5.5-4`), a namespace deliberately distinct from upstream `block/buzz`'s own `desktop-v*`/`relay-v*`/`chart-v*` release lanes — no collision, and this repo doesn't have upstream's real desktop-release workflow (it needs signing/App-token infra this fork lacks).
- **What's New splash — re-keyed off the real version (as of `0.5.5-4` → `0.5.5-5` transition)**:
  - The old `DEV_BUILD_LABEL`/`k2vN` cosmetic build-label system is retired. `desktop/src/features/whatsNew/changelog.ts`'s `WHATS_NEW_CHANGELOG` entries are now keyed by a plain number matching the release tag's trailing `-N` (e.g. entry `{ version: 5, ... }` surfaces on `0.5.5-5`).
  - `useWhatsNewModal.ts` reads the real running version via `@tauri-apps/api/app`'s `getVersion()` and compares its parsed trailing number against changelog entries — no separate identity to keep in sync.
  - To ship a new splash entry: bump the three version fields above, add one `{ version: N, bullets: [...] }` entry to `WHATS_NEW_CHANGELOG`, tag, push. Nothing else needs to change.
  - Historical entries `2`/`3`/`4` predate this scheme — they were three splash milestones bundled into the single first real release this fork shipped (`0.5.5-4`), before this repo had a working release pipeline.
- **Known critical config, easy to silently break**:
  - `tauri.conf.json`'s `bundle.createUpdaterArtifacts` must be `true` — without it the bundler produces no `.sig`/update-bundle artifacts and the updater is silently inert even though everything else looks configured.
  - `bundle.targets` is pinned to `["nsis"]` (not `"all"`) to avoid a live, unresolved Tauri bug where MSI bundling fails whenever `externalBin` is configured (tauri-apps/tauri#14681) — this repo has 6 externalBin sidecars, so it's directly affected.
  - `release.yml`'s `BUZZ_UPDATER_PUBLIC_KEY`/`BUZZ_UPDATER_ENDPOINT` build-time env vars gate whether `tauri_plugin_updater` compiles into the binary at all (`desktop/src-tauri/build.rs` → `cfg(buzz_updater_enabled)` → `lib.rs`). Must stay byte-identical to `tauri.conf.json`'s `plugins.updater.pubkey`/`endpoints`.
  - `desktop/src-tauri/tauri.windows.conf.json` (a Tauri platform-config override, auto-merged for Windows builds) trims `externalBin` to 5 entries, omitting `buzz-backend-kubernetes` — the Windows build never needs it built or placed.
  - `identifier` in `tauri.conf.json` (`xyz.block.buzz.app`) is still unchanged from upstream Block Buzz — a known, deliberately deferred issue. Windows derives the upgrade code/uninstall-registry entry from this value, so it collides with an official Buzz install if one is ever present on the same machine. Fixing it is a one-time breaking change (existing `0.5.5-4` installs won't upgrade in place) — not yet done, holding until there's a natural reason to eat that break.
  - `release.yml` mirrors `.github/workflows/windows-canary.yml` (the repo's own proven Windows Tauri build) for toolchain handling (no explicit Rust-install step — relies on `rust-toolchain.toml`'s pinned `1.95.0` via rustup auto-detection), `pnpm/action-setup@v4` pinned to `11.4.0` (not bare `corepack enable` — `desktop/package.json` has no `packageManager` field to pin to), and `CMAKE_POLICY_VERSION_MINIMUM: "3.5"` on both the sidecar build and the `tauri-action` build step.
- **First successful release**: `v0.5.5-4` published clean on the first real end-to-end CI run (all steps green, exactly 3 assets: `.exe`, `.sig`, `latest.json`). The in-app updater's "up to date" check against itself is expected, not proof of the update path — the real test is confirming an existing `0.5.5-4` install sees "update available" and successfully updates once `0.5.5-5` ships.
- **Signing**: `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repo secrets already set. Minisign keypair generated once; public half is baked into both `tauri.conf.json` and `release.yml`.

## Google Meet Integration (alternative to Huddle)
- **Why**: Huddle's voice quality is poor (custom relay-fanout audio, not WebRTC — see `huddle/mod.rs`). Adding real video to Huddle was explored and dropped (BuilderLab's hardcoded per-kind relay allowlist blocks any new event kind a WebRTC signaling path would need). Google Meet sidesteps that wall entirely: no new relay event kind is needed, since the join link is posted as an ordinary channel message — call audio/video is fully offloaded to Google's own infrastructure.
- **Model**: each user connects their own Google account (OAuth 2.0 + PKCE, `meetings.space.created` scope — principal-scoped, so a token only sees spaces it created itself). "Start a Google Meet" calls the Meet API (`POST https://meet.googleapis.com/v2/spaces`) to create an instant meeting space, then posts the join link as a normal message via the existing `useSendMessageMutation` — no huddle-style presence/relay-event mechanism.
- **Backend** (`desktop/src-tauri/src/google_meet.rs`): mirrors `builderlab.rs`'s `start_builderlab_login` loopback pattern (local `TcpListener` + tiny `axum` router + `oneshot` channel + `app.opener()`) rather than the `buzz://` deep-link scheme — deep links in this codebase are inbound-only and have no existing "browser round-trip" wiring. Refresh token is stored via `SecretStore::shared(keyring_service())` (the same OS-keyring store used for the Nostr identity), key `"google_meet_refresh_token"`. Commands: `start_google_meet_connect`, `cancel_google_meet_connect`, `get_google_meet_connection_status`, `disconnect_google_meet_account`, `create_instant_google_meet`.
- **Frontend**: `desktop/src/features/googleMeet/` — `hooks.ts` (react-query wrappers), `ui/GoogleMeetButton.tsx` (next to the Huddle button in `ChannelMembersBar.tsx`), `ui/GoogleMeetSettingsCard.tsx` (rendered in the existing Settings → Voice section, alongside `VoiceSettingsCard`).
- **Build-time config, same pattern as the updater's pubkey/endpoint**: `BUZZ_BUILD_GOOGLE_MEET_CLIENT_ID`/`BUZZ_BUILD_GOOGLE_MEET_CLIENT_SECRET` env vars, read via `option_env!` in `google_meet.rs` (not `env!` — unconfigured builds still compile, the feature just reports "not configured" at runtime). `build.rs` bakes them in as `BUZZ_DESKTOP_BUILD_GOOGLE_MEET_CLIENT_ID`/`_SECRET`. `release.yml` passes them from repo secrets of the **same names** (`BUZZ_BUILD_GOOGLE_MEET_CLIENT_ID`/`_SECRET`) — these must be added as GitHub Actions repo secrets before a release build actually has Google Meet working; until then the workflow still succeeds, the button just errors with "not configured for this build."
- **One-time Google Cloud Console setup needed (human-only, not automatable)**:
  1. In the existing k2alpha Google Cloud project, enable the **Google Meet API**.
  2. Create an OAuth 2.0 Client ID of type **Desktop app** (or reuse one if already created for the unrelated `google-sso` branch work — confirm its type is Desktop app, not Web, since the loopback-redirect flow this code uses is Desktop-app-specific and needs no pre-registered redirect URI/port).
  3. On the OAuth consent screen: if k2alpha's Google accounts are on a Google Workspace domain, set **User type = Internal** — this skips Google's verification requirement entirely for restricted/sensitive scopes like `meetings.space.created`. If accounts are personal Gmail (no Workspace domain), the client must be **External**; add each teammate's email under **Audience → Test users** (works immediately, no verification needed, just one click-through of an "unverified app" warning per user on first connect — fine for a small internal team, not fine for public distribution).
  4. Copy the Client ID (and Client Secret, if the Desktop app client type issued one) into the `BUZZ_BUILD_GOOGLE_MEET_CLIENT_ID`/`BUZZ_BUILD_GOOGLE_MEET_CLIENT_SECRET` GitHub Actions repo secrets.
- **Not yet done**: adding those two repo secrets (blocks the feature from actually working in a real release build until added), and a real end-to-end test (connect an account, start a meet, confirm the link posts and opens correctly).
