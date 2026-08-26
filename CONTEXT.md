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
  - `main` branch: this fork's release line. Carries all k2alpha work; periodically catches up to upstream `block/buzz` (`https://github.com/block/buzz.git`, remote `upstream`) via a content merge — see "Upstream catch-up" below. It is NOT a clean mirror of upstream.
  - `google-sso` branch: Dedicated fork branch preserving Google SSO peppered identity derivation & direct relay connection work.
- **Upstream catch-up (2026-08-26, `desktop-v0.5.18` → `desktop-v0.5.20` → `0.5.20-1`)**:
  - Two upstream releases in one pass (skipped 0.5.19): **76 commits, 22 files conflicted** — the same scale and shape as the 0.5.18 catch-up. Done on branch `integrate/desktop-v0.5.20` off `main`; a `backup/pre-0.5.20-integration` branch + a stash snapshot were taken first. **Verified fully green on Windows before go-live: `tsc` 0 errors, `pnpm test` 5,664 passing / 0 failing, `cargo check` clean (0 warnings).**
  - **Version is `0.5.20-1`, not `-0`.** `-0` is the bare catch-up point (the `desktop-v0.5.20` tag itself). This build stacks the whole fork set on top of that base — WhatsApp-style channel/mention toasts, the persistent channel file index, mention autocomplete, inbox read-state gating — so it is the *first fork build* on the new base. `-0` would only be correct for a pure catch-up carrying no new fork work.
  - **The send-signature merge is the one to get right (again).** Upstream added `rootEventId` and the fork has `supersedesTags` + `mentionScope`; all three are trailing params on `sendChannelMessage` (`tauriMessages.ts`) and all three named args exist on the Rust `send_channel_message`. Order in the JS signature: upstream's `rootEventId` at slot 13, then the fork's two after it — only `features/messages/hooks.ts` passes at those slots, so only it needed the call-site fix.
  - **`search_messages` moved AGAIN.** 0.5.20 relocated `search_messages`/`build_search_messages_filter`/`search_messages_limit` into a new `commands/messages/search.rs`. Keeping the copies inline in `commands/messages.rs` during the merge produced a *duplicate definition* that only `cargo check` caught (textual merge can't). Delete the inline copies + the now-unused `SearchResponse` import.
  - **MentionAutocomplete: take THEIRS, re-port ~31 lines.** Upstream did a big refactor of this component (options surface, `optionsOpen`, collision npub, `AtSign`/`ChevronRight`). The fork's `@channel`/`@here` scope suggestions are small and additive — take upstream's whole file, then re-add: `Megaphone` import, the `scope` suggestion-key line, the scope icon branch, the `@`-prefixed name/label, and the scope description line. (The `MentionSuggestion` type's scope fields auto-merge — they're outside the conflict.)
  - **SidebarSection: kept OURS** (the fork's unified unread *dot*) over upstream's new inline `UnreadCountBadge` — the fork renders the numeric badge elsewhere (absolute top-right), so upstream's would double it. This left `unreadCount` unused on `ChannelMenuButton` in `SidebarSection.tsx`/`CustomChannelSection.tsx` → remove the prop (TS6133).
  - **release.yml: kept OURS wholesale** — the fork deliberately replaced upstream's multi-platform workflow with a minimal Windows-only one (166 ins / 927 del). Taking theirs would revert that.
  - **Subscription-churn fix survived untouched.** The multiplexed single `#h:[all channels]` live subscription (`useLiveChannelUpdates.ts`) — the root-cause fix for the REQ storm that was flooding the relay rate limit and killing toasts/unread — was **not touched by upstream between 0.5.18 and 0.5.20**, so it wasn't even a conflict. 0.5.20's new `channel_head_cache` module is a *local* SQLite cache, not a relay subscription, so it can't reintroduce the churn.
  - **Dead code removed this pass** (audited, unreferenced-except-tests): `canUseMentionScope`, `resolveMentionAudience`, `mentionScopeTag`, `CHANNEL_MENTION_ADMIN_THRESHOLD`, orphaned `PRIVILEGED_ROLES` (all in `globalMentions.mjs` + `.d.mts` + test), and `buildChannelAuxFilter` in `relayChannelFilters.ts`.
  - **Still open: DM toast.** Channel + top-level-mention toasts fire; DMs do not. Temporary diagnostic scaffolding is deliberately KEPT in this build (`relay_debug_log.rs`, the `NOTIFY-DM`/`DM-EVENT`/`TOAST`/`NOTIFY-FEED`/`REQ` loggers, `logDebug`, `BUILD=` tag) so a live DM test produces the log lines that pinpoint the gate. **Strip all of it as one unit before a clean release** — see the notifications KI for the exact line map.
- **Upstream catch-up (last done 2026-08-18, `desktop-v0.5.17` → `0.5.17-0`)**:
  - 7 commits, 6 conflicts, all version/metadata. `relay_directory.rs` and `lib.rs` both auto-merged cleanly *and* were correct this time — `cargo check` confirmed rather than assumed. Upstream shrank `lib.rs` to 999 lines, so our 1013 cleared the ratchet without work.
  - **The file-size ratchet is now a `pre-push` lefthook hook** (upstream #6187), so a ratchet failure blocks `git push` rather than only failing a manual check. Run `check:file-sizes` deliberately before pushing a catch-up. Their change to `check-file-sizes-core.mjs` is only a `GIT_*` env-stripping fix for hook contexts — baseline logic is unchanged.
  - **Correction to the `0.5.16-0` note below**: the document-preview dependencies (`mammoth`, `pdfjs-dist`, `xlsx`, `@jvmr/pptx-to-html`, `dompurify`) are **ours, not upstream's**. They are absent from `desktop-v0.5.16`'s `package.json` and from the merge-base; `pnpm install` was re-reporting top-level deps after `package.json` changed, not adding new ones. Upstream has **no** document preview — their `*Preview*` files are link previews, agent cards and image tool previews. All of `shared/ui/filePreview/` is fork-only, so there is nothing to reconcile.
  - To settle "is this ours or theirs" for any path: `git ls-tree -r --name-only desktop-v<tag> -- desktop/src | grep <name>`. Present at the tag means theirs; only at `HEAD` means ours; in both, `git diff desktop-v<tag> HEAD -- <path>`.
- **Upstream catch-up (2026-08-17, `desktop-v0.5.16` → `0.5.16-0`)**:
  - 23 commits, **6 conflicts, all version/metadata files** (`package.json`, `tauri.conf.json`, `Cargo.toml`, `Cargo.lock`, `.release/desktop-candidate.json`, `CHANGELOG.md`). Resolution is the same each time: take upstream's structure, then set our own `<core>-0`. Skipped `0.5.15` and went straight to `0.5.16` — they were 3 commits and hours apart, and stopping at the older one just means merging again immediately.
  - **`events.rs` and `tauri.ts` auto-merged, which was not the same as being correct.** Upstream added a *new* `build_message` caller (`commands/agent_discovery/relay_directory.rs`) that knew nothing about our two extra trailing params (`supersedes_ref_tags`, `mention_scope`). Textual auto-merge cannot catch an arity mismatch — only `cargo check` did. **After any catch-up, grep for every `build_message` call site.**
  - **The file-size ratchet fails on a catch-up and it is not a false alarm.** It baselines on `merge-base(origin/main, HEAD)` — locally, and `HEAD^1` in CI — so upstream's growth of an already-over-limit file reads as *our* growth. Here upstream took `lib.rs` from 1016 to 1018. Fixed properly by extracting: `ensure_startup_registration` became a cfg-gated pair in `commands/notifications.rs`, so `lib.rs` calls it unconditionally instead of wrapping it in a 4-line `#[cfg]` block. Same shape as `mouse_nav.rs`. Do not reach for `CHECK_FILE_SIZES_BASE` — CI will not have it.
  - `pnpm install` before trusting `tsc`, since upstream changed `package.json`. (An earlier version of this note claimed upstream had added the document-preview dependencies — that was wrong; see the `0.5.17-0` entry above.)
  - **`mobile/` is a full Flutter app under active development** (its own `mobile-v*` release lane, currently `0.11.0-rc.2`). We have never built or shipped it. Its release workflow only tags — build and TestFlight distribution live inside Block and cannot be reproduced from this repo.
- **Upstream catch-up (2026-08-15, `desktop-v0.5.14` → `0.5.14-0`)**:
  - The fork sat 9 versions / 141 commits behind before this. Catching up was a full `git merge` of upstream's `desktop-v0.5.14` tag, not a cherry-pick series — one fork commit (`71ed371a`) was mislabelled as a CI tweak while actually carrying 68 files of unrelated feature work, which made per-commit replay impractical.
  - 22 files conflicted. The fork's own auto-updater wiring was dropped in favour of upstream's (`updater.rs`/`use-updater.ts` now exist upstream); only the fork's GitHub Releases endpoint config was kept. Upstream's new multi-platform release workflow was rejected in favour of the fork's single-job Windows one, because upstream's hardcodes publishing to `block/buzz` and depends on sibling jobs this repo doesn't have.
  - **Two independent Cargo workspaces**: root `Cargo.toml` (`exclude = ["desktop/src-tauri"]`) and `desktop/src-tauri/Cargo.toml` (`members = ["crates/buzz-terminal"]`). `cargo check --workspace` must be run from BOTH directories — running it in one place silently skips the other's crates.
- **Native Windows Notifications (Fixed in `0.5.18-2` & `0.5.18-3`)**:
  - Webview2 `window.Notification` on Windows 10/11 Action Center fails silently without AUMID registration.
  - Native Windows Toast Notifications use `tauri-winrt-notification` (`Toast::new(&app_id)`) in `desktop/src-tauri/src/commands/notifications.rs` and `isWindowsPlatform()` in `desktop/src/features/notifications/lib/desktop.ts`.
  - `Toast::show()` requires a COM apartment. Because Buzz posts from a bare `std::thread`, calls previously failed silently with `CO_E_NOTINITIALIZED` (`0x800401F0`). Fixed in `0.5.18-2` by establishing a permanent process-wide MTA on a dedicated parked thread (`ensure_process_mta`), which background threads join implicitly.
  - In `0.5.18-3`, startup registration also pre-registers `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\<aumid>` (`ShowInActionCenter=1`, `Enabled=1`) so Buzz appears in Windows Settings > System > Notifications immediately upon launch.
  - `Toast::text1` and `Toast::text2` consume `self` by value; builder methods must reassign instances (`toast = toast.text2(...)`).
- **Desktop notification routing (as of `0.5.20-1`) — four paths, one delivery primitive**:
  - All desktop toasts end at `sendDesktopNotification` → the Rust `windows::show` toast. The OS-delivery layer is proven working (channel + mention toasts fire, Buzz is registered in Windows Settings with banners on).
  - Paths: (a) **channel messages + top-level @mentions** via the live path `handleChannelNotification` in `useAppShellDesktopNotifications.ts` — WhatsApp-style: every top-level message in an *unmuted* channel toasts (muted channels excluded upstream by `shouldNotifyForEvent`); top-level mentions are routed here too (mention-specific copy) rather than the home-feed path, because the feed path was unreliable (`eligibleNew=0`). (b) **DMs** via `handleDmEvent` → `onDmMessage` → `handleDmNotification`. (c) **thread replies** via `handleThreadReplyDesktopNotification`. (d) home-feed mentions/needs-action via `useFeedDesktopNotifications` (kept, but the live path is the reliable one).
  - **DM toast is the open bug.** Channel/mention toasts work; DMs do not, despite the DM path being fully wired and the DM slot defaulting on. The `NOTIFY-DM` (in `handleDmNotification`) + `DM-EVENT` (in `handleDmEvent`) loggers will name the exact failing gate on the next live DM test (send to a DM you are NOT viewing; `active-channel` bail is the common false alarm).
  - **Temporary debug scaffolding to remove before a clean release (one unit, remove together):** the Rust module `src-tauri/src/relay_debug_log.rs` (`append_line`, `#[tauri::command] debug_append_relay_log`) + its `mod`/registration in `lib.rs`; the 3 `TOAST` `append_line` calls in `commands/notifications.rs`; `logDebug` + the `BUILD=` tag + `REQ`/EOSE/CLOSED loggers in `shared/api/relayClientSession.ts`; the `dmLog` helpers (`NOTIFY-DM`) in `useAppShellDesktopNotifications.ts` and (`DM-EVENT`) in `useLiveChannelUpdates.ts`; the `NOTIFY-FEED` `invoke` in `use-feed-desktop-notifications.ts`; and the `NOTIFY` `invoke`s in `notifications/lib/desktop.ts`. All are self-labelled `TEMPORARY … remove before release`.
- **Live-subscription multiplexing (root-cause fix for the toast/unread outage) — do not regress**:
  - Background channels used to open one live subscription *per channel*; any change to the channel set (including the transient flicker a channels refetch produces on every relay reconnect) tore down and recreated ALL of them. The relay frame log showed ~40 re-subscribes/channel (839 REQs in 100s), which blew the relay WS rate limit (default 50 frames / 5s), got subscriptions `CLOSED`, and dropped live events — so background channels never lit up (dead unread / Inbox / tray / toast).
  - Fixed by collapsing to **ONE multiplexed subscription** with a single `#h:[all joined channels]` filter, chunked at `LIVE_SUB_CHANNELS_PER_FILTER = 100` (relay caps explicit `#h` values per filter at 128, `MAX_EXPLICIT_CHANNEL_VALUES`). A set-change now costs one frame instead of N. Events route by their own `h` tag in `handleIncomingMessage`, so which filter delivered one is irrelevant. Lives entirely in `features/channels/useLiveChannelUpdates.ts` — a fork-owned file upstream has not touched, so a future catch-up that conflicts here is the signal to re-preserve it.
  - A pacing band-aid was tried and REVERTED — it *amplified* churn (280→839 REQs) via a cancel-before-store race. Multiplexing, not pacing, is the fix.
- **Cross-Platform Target Gating (`mouse_nav.rs`)**:
  - macOS-only AppKit/Block modules (`block2`, `objc2_app_kit`) must be gated with `#[cfg(target_os = "macos")]` and provide a `#[cfg(not(target_os = "macos"))]` no-op stub (`pub fn init`) to maintain clean cross-platform builds on Windows and Linux.
- **GitHub Actions Workflow Parameters**:
  - `actions/setup-node` input parameter is `cache`, not `package-manager-cache`. Invalid parameter keys trigger workflow step validation failures.
- **File-size ratchet (`pnpm check:file-sizes`, 1000-line cap) — two files are at the wall**:
  - `desktop/src/features/home/ui/HomeView.tsx` (998) and `desktop/src/features/channels/useUnreadChannels.ts` (994) as of `0.5.14-2`. Adding even a few lines to either will fail the check.
  - Both carry `// biome-ignore format: file size ratchet` directives where declarations were squashed onto single lines to fit. In `HomeView.tsx` four of these sit on code unrelated to the change that needed the room (`latchedDefaultParentId`, `selectedChannelIdCandidate`, `selectedChannel`, `managedChannel`) — they read as arbitrary unless you know why.
  - **When these next fail, extract rather than squash.** Squashing buys a line or two and makes the file worse; the honest fix is moving a cohesive group out (in `HomeView.tsx`, the three channel-selection memos are the obvious candidate and would free ~10 lines).

## Pending Tasks
- **Google SSO Onboarding Race Condition**:
  - `useCommunityInit.ts` auto-connects to default relay on initial mount before Google SSO runs when `identity.storage === "ephemeral"`.
  - Fix: Guard `useCommunityInit` to skip `initFirstCommunity` when `identity.storage === "ephemeral"`, allowing Google SSO to derive and import the true identity first.
- **`0.5.14-1` behaviour checks outstanding**: the release passed typecheck, lint, 4,978 frontend tests and both `cargo check`s, but the manual behaviour checks were not run — notably the file-deletion round-trip (deleting either side of a version pair should clear the other's badge), which was reproduced as broken on `0.5.14-0`.
- **Public-facing docs added in `0.5.14-4`, unpushed until then**: `FORK.md` (what the fork adds, plus its limitations), a README banner pointing at it, `docs/roadmap-and-research.md` (open items with the community research behind them) and `docs/upstream-candidates.md` (what should go back to `block/buzz`, in three tiers). The highest-leverage follow-up is **opening the Tier 1 upstream PRs** — two self-contained bug fixes — which needs a human to submit.
- **In-app Help panel (Settings → App) not built**: the *channel-based* Help shipped instead (see "Help agent" below) and covers the need for now. A Settings panel would still need a one-shot command that bypasses channels, since the managed-agent path always posts into one.
- **Google Drive routing has never run against a real Google account.** It typechecks and its rules are unit-tested; the OAuth scope change, the resumable upload, the folder creation and the 308 handling are all unexercised. First real test is one person sending one video. Note that **every existing connected account must disconnect and reconnect once** to grant `drive.file`.
- **The mp4 upload failure that motivated the Drive work is still undiagnosed**, and is now largely moot — video no longer takes that path. Worth an upstream issue rather than an investigation. The message was unhelpful because of the `getErrorMessage` bug in `docs/upstream-candidates.md`.
- **Inbox behaviour checks outstanding**: the redesign passed typecheck, lint and 4,992 tests, but was never exercised against a running relay. Specifically unverified: an unmentioned unread channel appears; a muted channel does not; reading in the channel clears the Inbox row and vice versa; the count matches the sidebar badge; a channel with a mention yields exactly one row, not two.

## k2alpha Auto-Updater & Release Pipeline
- **Distribution model**: `.github/workflows/release.yml` builds, signs, and publishes a Windows NSIS installer + Tauri updater manifest to GitHub Releases on this fork (`github.com/ranjank2alpha/buzz`, public repo) whenever a `v*` tag is pushed. Existing installs poll `https://github.com/ranjank2alpha/buzz/releases/latest/download/latest.json` (configured in `desktop/src-tauri/tauri.conf.json`'s `plugins.updater.endpoints`) every 6 hours (`desktop/src/features/settings/hooks/use-updater.ts`).
- **Versioning scheme**:
  - The core version tracks whatever upstream release the fork has last caught up to — `0.5.5` until 2026-08-15, then `0.5.14`, `0.5.16`, `0.5.17`, `0.5.18`, now `0.5.20` (2026-08-26). It changes ONLY as part of an upstream catch-up merge, never on its own.
  - k2alpha releases use a semver pre-release suffix: `0.5.14-0`, `0.5.14-1`, ... The number after the dash increments per k2alpha release, resets to `0` on each upstream catch-up, and must stay purely numeric (Tauri's Windows/WiX bundler only accepts a numeric pre-release segment).
  - Bump this version string in exactly three places together: `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/Cargo.toml`, `desktop/package.json`.
  - Tag format is `v<version>` (e.g. `v0.5.14-1`), a namespace deliberately distinct from upstream `block/buzz`'s own `desktop-v*`/`relay-v*`/`chart-v*` release lanes — no collision, and this repo doesn't have upstream's real desktop-release workflow (it needs signing/App-token infra this fork lacks).
  - **The suffix is not monotonic across catch-ups** (`0.5.5-5` → `0.5.14-0` goes *down*). Nothing may derive ordering or identity from it — see the splash note below for what happens when something does.
- **What's New splash — keyed on the full version string (as of `0.5.14-1`)**:
  - `desktop/src/features/whatsNew/changelog.ts`'s `WHATS_NEW_CHANGELOG` entries are keyed by the complete version string (`{ version: "0.5.14-1", bullets: [...] }`), matched whole by `changelogEntryForVersion`. Position in the array is the release order; nothing is parsed out of the string and no comparison is done on it.
  - **Why**: the previous scheme parsed the trailing `-N` and showed every entry `<= N`. The `0.5.5-5` → `0.5.14-0` renumbering reset that counter to 0, no entry matched, and the splash silently stopped appearing — it was absent for the whole `0.5.14-0` release before anyone noticed. It would then have come back *wrong*: `0.5.14-2` would have re-shown the year-old entry `2` as new. A hardcoded `0.5.5-` prefix in `WhatsNewModal.tsx` compounded it.
  - The splash shows ONLY the running version's entry, plus a pointer to Settings → Updates. The full version-by-version history lives there (`features/whatsNew/ui/ReleaseHistory.tsx`, rendered inside `UpdateChecker`), so the modal cannot grow a section per release.
  - A version with no changelog entry shows no splash at all. **This means forgetting to add an entry silently disables the splash for that release** — add the entry before tagging.
  - To ship a new splash entry: bump the three version fields above, append one `{ version: "<full version>", date: "<YYYY-MM-DD>", bullets: [...] }` entry to `WHATS_NEW_CHANGELOG`, tag, push. **Set the date** — it orders the Settings → Updates history, and an undated entry sinks to the bottom.
  - **Same-day releases tiebreak on array position, not version string** (`localReleaseRows` emits `order`). `compareVersions` strips the prerelease suffix by design — it exists to compare against upstream's ceiling — so `0.5.14-4` and `0.5.14-5` compare *equal* and the stable sort silently left them oldest-first. Shipped backwards in `0.5.14-5` before anyone noticed.
  - **Repeating the previous release's bullets is sometimes right.** The splash shows only the running version's entry, so a user going `-3 → -5` never sees `-4`'s. When two releases land close together, fold the earlier bullets into the later entry (`0.5.14-5` does this).
- **Known critical config, easy to silently break**:
  - `tauri.conf.json`'s `bundle.createUpdaterArtifacts` must be `true` — without it the bundler produces no `.sig`/update-bundle artifacts and the updater is silently inert even though everything else looks configured.
  - `bundle.targets` is pinned to `["nsis"]` (not `"all"`) to avoid a live, unresolved Tauri bug where MSI bundling fails whenever `externalBin` is configured (tauri-apps/tauri#14681) — this repo has 6 externalBin sidecars, so it's directly affected.
  - `release.yml`'s `BUZZ_UPDATER_PUBLIC_KEY`/`BUZZ_UPDATER_ENDPOINT` build-time env vars gate whether `tauri_plugin_updater` compiles into the binary at all (`desktop/src-tauri/build.rs` → `cfg(buzz_updater_enabled)` → `lib.rs`). Must stay byte-identical to `tauri.conf.json`'s `plugins.updater.pubkey`/`endpoints`.
  - `desktop/src-tauri/tauri.windows.conf.json` (a Tauri platform-config override, auto-merged for Windows builds) trims `externalBin` to 5 entries, omitting `buzz-backend-kubernetes` — the Windows build never needs it built or placed.
  - `identifier` in `tauri.conf.json` (`xyz.block.buzz.app`) is still unchanged from upstream Block Buzz — a known, deliberately deferred issue. Windows derives the upgrade code/uninstall-registry entry from this value, so it collides with an official Buzz install if one is ever present on the same machine. Fixing it is a one-time breaking change (existing `0.5.5-4` installs won't upgrade in place) — not yet done, holding until there's a natural reason to eat that break.
  - `release.yml` mirrors `.github/workflows/windows-canary.yml` (the repo's own proven Windows Tauri build) for toolchain handling (no explicit Rust-install step — relies on `rust-toolchain.toml`'s pinned `1.95.0` via rustup auto-detection), `pnpm/action-setup@v4` pinned to `11.4.0` (not bare `corepack enable` — `desktop/package.json` has no `packageManager` field to pin to), and `CMAKE_POLICY_VERSION_MINIMUM: "3.5"` on both the sidecar build and the `tauri-action` build step.
- **Release history**: `v0.5.5-4` was the first clean end-to-end CI run (3 assets: `.exe`, `.sig`, `latest.json`). `v0.5.14-0` (the upstream catch-up) and `v0.5.14-1` followed on the same pipeline. Tag-push → published release takes roughly 50 minutes (Rust + 5 sidecars).
- **Auto-update path still unproven end to end**: an install checking against its own version reports "up to date", which proves nothing. The real test is an existing install seeing "update available" and successfully applying it — first genuine opportunity is a `0.5.14-0` install offered `0.5.14-1`.
- **Signing**: `TAURI_SIGNING_PRIVATE_KEY`/`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repo secrets already set. Minisign keypair generated once; public half is baked into both `tauri.conf.json` and `release.yml`.

## File Versioning ("supersedes")
- **Model**: a file message may carry `["e", "<older-event-id>", "", "supersedes"]`, declaring its attachment a newer version of an earlier upload. The link is a tag on the file's own message — there is no separate link event, and no way to add a link after the fact.
- **Set only at upload time.** The composer (`MessageComposer.tsx` → `ComposerAttachments.tsx`) prompts on every attachment. A mistake is undone by deleting the message, which takes the link with it. This is why there is no "edit/remove version tag" feature: there is nothing to edit.
- **Retroactive linking was removed in `0.5.14-1`, deliberately — do not reintroduce it naively.** `build_supersedes_link` published the link as an **empty-content kind:9 event**, and kind 9 is the ordinary channel-message kind. It had to be a timeline kind because `listChannelFiles` can only discover events the relay returns for `TIMELINE_KINDS` — so making the link *discoverable* also made it *renderable*, and every version tag posted a blank message to the channel under the tagger's name. `isSupersedesLinkDeclaration` (in `shared/api/supersedesTags.ts`) still filters those out of the timeline, and `channelFiles.ts` still *reads* legacy link events, so links published by older builds keep resolving.
- **The relay soft-deletes.** A deleted message keeps coming back from `/query`, so deletion state cannot be inferred from absence. `listChannelFiles` reads the relay's kind:40099 tombstones (`{"type":"message_deleted","target_event_id":"<hex>"}`, emitted by `handle_delete_event` in `crates/buzz-relay/src/handlers/side_effects.rs`) — kind 40099 is already in `TIMELINE_KINDS`, so they arrive in the pages already being walked. Without this, deleting a file left it in the Files tab asserting its link forever (observed on `0.5.14-0`).
- **Derived, never stored.** A message cannot know it has been superseded — that fact lives in a *later* message. Every badge, chain and position is recomputed at read time from the channel-wide file list, which is why `FileVersionProvider` (`shared/context/FileVersionContext.tsx`) exists: it supplies the graph to attachment cards that only know their own URL. It is mounted inside `MessageTimeline` and its query is demand-driven — `listChannelFiles` pages the channel's entire history, so it only runs while a file card is actually on screen.
- **Chain rules**: only the newest version carries the "Supersedes N earlier versions" disclosure; older ones carry "Outdated — view latest", which jumps to the *head* of the chain, not one step. Same rule in the Files tab (one row per chain, older versions collapsed under it). Deleting a middle version splits the chain — the link between the ends only ever existed inside the deleted message's tag.
- **Pure, tested logic** lives in `features/messages/lib/`: `fileVersionChains.mjs` (grouping, latest-resolution; handles cycles, forks, self-supersedes, missing parents) and `supersedesRanking.mjs` (fuzzy filename matching, 62-day candidate window, refuses to pre-select on a tie). Both are `.mjs` with hand-written `.d.mts` siblings so `node:test` exercises the exact source the UI runs — same pattern as `applyEditTagOverlay.mjs`.

## Links as file entries (`0.5.17-1`, Phase 1 of the Drive spec)
- **Any http(s) URL in a channel message is an entry in that channel's Files tab**, in the same list and the same version chains as uploaded files. `shared/lib/channelLinkEntries.mjs` (+ `.d.mts`, 24 tests) does the pure work; `channelFiles.ts` merges the result into `files` *before* the supersedes passes, so links get chain resolution, tombstone handling and the composer's supersedes picker for free.
- **Why this was cheap**: the supersedes tag references an **event**, not a file. Nothing in the chain machinery ever cared whether the entry behind an event id was an upload. This is the single reason Phase 1 was worth doing before any Google work — see `docs/google-drive-integration-spec.md`.
- **`ChannelFileEntry` gained `kind: "file" | "link"`.** Nothing constructs that type outside `channelFiles.ts`, so adding a required field was safe; check that again before adding another.
- **Two rules that are not obvious and will look like bugs if removed**:
  - **A URL already present as a file's own `url` never becomes a link entry.** The markdown renderer embeds an attachment's URL in the message body, so without the exclusion every upload also produced a duplicate link row beside itself.
  - **A supersedes tag reaches a link only when its message has exactly one link and no attachment.** A message can carry a file *and* a link under one event id; the tag belongs to the file. The guard exists in two places — `collectChannelLinkEntries` at collection time, and the `kind !== "file"` skip in `channelFiles.ts`'s "apply merged map" pass, which would otherwise re-apply it and undo the first guard.
- **Naming is the weak part, and known to be.** The document's title is not in the event and link previews are fetched at render time (`fetch_link_preview_metadata`), never stored. So naming falls back to: Google surface by kind (`Google Doc`, `Google Drive file`), then the last non-opaque path segment, then the host. Opaque segments (≥16 id-safe chars, or ≥6 digits) are skipped rather than shown — a bare Drive id in a file list is worse than the host. **Phase 2 replaces step one with the real Drive title.**
- **Dedupe is by normalized URL** (host lowercased, `www.` and fragment and trailing slash dropped, query string **kept** — `?gid=` selects the sheet). One row per link, dated at its *earliest* appearance, with ties broken by event id so the result does not depend on relay paging order.

## `0.5.18` catch-up — resolution plan (audited 2026-08-21 by trial merge + five parallel audits)
Method: `git worktree add --detach /tmp/x HEAD; git merge --no-commit desktop-v0.5.18` in a throwaway worktree. **Do this before every catch-up.** Line-count heuristics were wrong five times in this audit; the trial merge plus reading the actual diffs was right every time.

**15 conflicted files.** Merge base is `3fdf289b7`, which is why the release-metadata files conflict.

| File | Resolution |
|---|---|
| `shared/api/tauri.ts` | **Take THEIRS** — upstream *moved* `sendChannelMessage` into a new `shared/api/tauriMessages.ts` and re-exports it, so deleting our copy is correct and no importer breaks. `getPinnedMessages`/`setPinnedMessages` auto-merge and survive. **Then see the row below.** |
| `shared/api/tauriMessages.ts` (not conflicted — upstream-only new file) | **Re-apply our two params here.** Upstream took positional slots 11/12 for `expectedRelayUrl`/`expectedSignerPubkey`, which is where ours used to sit. Append `supersedesTags` and `mentionScope` at **13/14**, and fix the single fork caller `features/messages/hooks.ts:545` by inserting `undefined, undefined,` after `sentFromThreadTag`. The other 6 call sites pass ≤10 args and need nothing. |
| `src-tauri/src/lib.rs` | **Take THEIRS for the conflict block only** — it is one block containing *just* the `generate_handler!` list. Every other fork change to `lib.rs` already auto-merged. Then delete `mod command_registry;` and the file `command_registry.rs`, and re-add **11** fork command entries: 7 `google_meet::*` (incl. `drive::get_google_drive_status`, `drive::upload_drive_bytes_raw`), `get_pinned_messages`, `set_pinned_messages`, `check_libreoffice_available`, `convert_pptx_to_pdf` — plus delete the four `#[cfg(target_os = "macos")]` lines above the tray entries (the fork made the tray cross-platform). No import changes needed; all resolve via `use commands::*`. **Result: 948 lines.** |
| `features/channels/useUnreadChannels.ts` | **Take THEIRS.** See the regression note below — the plan to "re-apply `{ presence: "offline" }`" is **impossible**. The `biome-ignore format: file size ratchet` squash auto-merges and must be preserved (992 lines with it, 997 without). |
| `features/messages/ui/MessageTimeline.tsx` | **Take OURS**, then change `<TooltipProvider delayDuration={200}>` to `<TooltipProvider>` — that one line is upstream's entire change to this file. |
| `app/AppShell.tsx` | **Take THEIRS**, then re-add one line: `{!isHuddleRoom ? <WhatsNewModal /> : null}`. The other two fork lines auto-merge. Resolved = 994 lines, only 6 of ratchet headroom. |
| `features/channels/ui/ChannelPane.tsx` | **Take THEIRS** (upstream added a stacking-context wrapper, reindenting by +2), then re-insert the `PinnedMessagesBar` mount after `{isHuddleTranscript ? null : header}`. |
| `src-tauri/src/commands/messages.rs` | 3 hunks. **Hunk 1: take theirs MINUS `nostr_convert`** — the fork moved `search_messages` out to `commands/messages/search.rs`, so that import would be unused and CI runs `clippy -D warnings`. **Hunks 2 and 3: union**, keep both sides (Tauri args are by name, so order is irrelevant). |
| `managed_agents/discovery.rs` + `discovery/tests.rs` | **NOT rustfmt conflicts** — they are load-bearing **Windows clippy fixes** from `71ed371a5`. `discovery.rs`: take theirs, then re-apply `return None;` → `None` at the new `discovery/login_shell.rs:89`. `discovery/tests.rs`: take theirs, then delete four names from the `use super::{…}` list whose only uses are `#[cfg(unix)]`-gated — `codex_adapter_availability`, `codex_adapter_is_outdated`, `find_nvm_default_bin`, `probe_codex_acp_version`. Skipping either breaks Windows CI. |
| `CHANGELOG.md`, `.release/desktop-candidate.json` | Take theirs verbatim. The candidate JSON keeps upstream's plain `0.5.18`, not `0.5.18-0`. |
| `package.json`, `Cargo.toml`, `tauri.conf.json` | Neither side → `0.5.18-0`. |
| `Cargo.lock` | **A FOURTH version location**, at `[[package]] name = "buzz-desktop"` → `0.5.18-0`. Miss it and the next `cargo` run dirties the tree. |

### The one real behavioural regression — decide before merging
0.5.18 **deleted the entire JS unread catch-up loop** and moved it to Rust (`src-tauri/src/unread_catch_up.rs`). Our `{ presence: "offline" }` argument lived inside the deleted block; there is nothing to re-apply it into. Upstream's Rust classifier is:
```rust
let high_priority = item.channel.channel_type == "dm" || broadcast
    || has_tag_value(&event.tags, "p", &self_pubkey);
```
It knows nothing about our `["mention-scope", "channel"|"here"]` tag. Consequences:
- **`@here` is never high-priority on catch-up** — which is accidentally exactly what we wanted. No action.
- **`@channel` is never high-priority on catch-up either — that is a real regression**, and it erases the entire `@channel`-vs-`@here` distinction FORK.md advertises. Minimal fix: add `|| has_exact_tag(&event.tags, "mention-scope", "channel")` to that expression.
- Secondary: upstream's `should_notify` (same file) has no mention-scope branch, so a threaded reply carrying `@channel` in a thread you have not participated in is filtered out of catch-up entirely. Follow-up, not a merge blocker.

### Ratchet extractions of upstream test modules break on every catch-up
Three separate failures in the `0.5.18` merge, all the same shape, and **none of
them visible to `cargo check`** — only `cargo clippy --workspace --all-targets`,
because test code is not compiled otherwise.

Upstream keeps its Rust tests inline in a `mod tests` at the bottom of the file
under test, where they see the parent module's private items and make their own
local imports. This fork has extracted several of those test modules into
sibling files to satisfy the 1000-line ratchet. Those files start with
`use super::*`, which reaches the parent's **module-level** imports only — not
the imports upstream makes *inside* its test block, and not private submodules
the parent does not re-export.

So every upstream release that adds a test lands it in our extracted file, where
it cannot see what it saw upstream. What broke here:

- `events_tests.rs` — three new upstream tests using `Keys`, imported inside
  upstream's inline `mod tests`. Fixed with `use nostr::Keys;`.
- `commands/messages_tests.rs` — a new test calling
  `build_search_messages_filter`, which this fork moved into a private
  `commands/messages/search.rs` that `messages.rs` does not re-export. Fixed with
  `use super::search::build_search_messages_filter;`.
- `managed_agents/discovery.rs` — a `#[cfg(test)]` re-export of
  `login_shell_candidates` whose only consumer is a `#[cfg(unix)]` test, so it
  is an unused import on Windows under `-D warnings`. Fixed by narrowing the
  re-export to `#[cfg(all(test, unix))]`.

**Always run `cargo clippy --workspace --all-targets -- -D warnings` from
`desktop/src-tauri` after a catch-up.** `cargo check` passing means nothing here.
Files carrying this liability today: `events_tests.rs`,
`commands/messages_tests.rs`, `commands/messages/search.rs`,
`managed_agents/discovery/tests.rs`.

### Upstream's Unix-gated test scaffolding is dead code on Windows
A second, unrelated clippy failure class, same catch-up. Upstream adds test
helpers whose only callers are `#[cfg(unix)]` tests. **Upstream's CI does not
build this crate for Windows; ours does**, so their `-D warnings` passes and
ours does not. In `0.5.18`: `discovery/auth_status_cache.rs::len`, and
`discovery/login_shell_spawn_probe.rs::{reset, count}`.

Fix with `#[cfg_attr(windows, allow(dead_code))]`, not a blanket
`#[allow(dead_code)]` — the lint should stay live on Unix, where genuine
deadness still matters. Do not gate the function itself on `#[cfg(unix)]`: the
callers are Unix-gated, the definition is not, and hiding it would break
upstream's own structure for no gain.

This is a permanent tax on being a Windows-first fork of a project that is not,
and it will recur on any release that adds Unix-only test scaffolding. It is
**not** the same as the extraction problem above, and the fix is different.

### `packageManager` drift breaks the release build
The **root** `package.json` carries `"packageManager": "pnpm@<v>"`, and
`.github/workflows/{release,windows-canary}.yml` pin the same version in
`pnpm/action-setup`. **A mismatch is a hard error** —
`ERR_PNPM_BAD_PM_VERSION` — and it fails the release build before anything is
compiled.

**Running `pnpm install` locally with a newer pnpm rewrites that field.** It
then rides into whatever commit `git add -u` next sweeps up. That is exactly how
the `v0.5.18-1` build broke: local pnpm was 11.22.0, upstream's pin is 11.4.0.

Two habits: after any `pnpm install`, run `git diff package.json` before
staging; and note this lives in the **root** `package.json`, not
`desktop/package.json`, which has no such field. An earlier version of this
document and the `release.yml` comment both said there was no `packageManager`
field anywhere — that was true before `desktop-v0.5.18` and is now wrong.

### 0.5.18 raises the minimum local Node version
Upstream's new `features/projects/ui/useRetainedProjectGitViews.test.mjs` imports
`registerHooks` from `node:module`, which exists only in **Node 22.15+**. On an
older local Node the whole test file fails to parse and `pnpm test` reports one
failure that has nothing to do with any change. CI is unaffected —
`release.yml` pins Node 24. **Do not patch upstream's test**; upgrade Node. It
is the only file in the repo using that API, so this will not recur until
upstream adopts another new-Node feature.

### Also found
- **New upstream test helpers call bare `bash`** — `pool.rs` merged lines ~8579, ~8750, ~8839, ~9135 (`spawn_effort_acp`, `spawn_switch_acp`, etc.). Our fork added a `#[cfg(windows)]` shim for exactly this. They will fail on a Windows dev box. Cheapest fix: hoist the shim into one `fn test_bash_cmd() -> &'static str` and point all call sites at it.
- **`useLiveChannelUpdates.ts`: we never modified it.** Straight fast-forward. But upstream now calls `updateChannelLastMessageAt` on *every* unread-trigger event including self-authored and muted, so anything rendering off the channels cache (sidebar rollup, Inbox lanes) re-renders per inbound message. Correct, just noisier.
- **`shared/ui/markdown.tsx`**: our fork inlined a private copy of `getImageLightboxFocusableElements` instead of importing it. Verified identical today, but it will not receive upstream fixes. Consider re-importing and deleting the copy.
- **`commands/messages/search.rs` is a shadowing hazard**: it is upstream code the fork extracted for the ratchet, byte-identical at both tags. Any future upstream change to `search_messages` lands in `messages.rs` and our copy silently shadows it.
- **False alarm, recorded so it is not re-investigated**: `features/local-archive/archiveSyncManager.ts` appears "fork-only" under `git diff --diff-filter=A desktop-v0.5.18 HEAD` because that direction lists *upstream deletions* as our additions. The fork never touched local-archive. The merge deletes the file correctly and `AppShell.tsx` follows upstream to `useArchiveSync`. **`--diff-filter=A` against a newer ref is not a fork-ownership test.**

### Auto-merged but NOT verified
Clean text is not correct behaviour. `features/channels/hooks.ts` (198 vs 49 — audited SAFE, changes are disjoint), `shared/ui/markdown.tsx` (audited SAFE), `features/notifications/lib/desktop.ts` (audited SAFE, zero function overlap), `crates/buzz-acp/{acp,lib,pool}.rs` (audited SAFE; covered by the **root** `cargo check --workspace`, the one that gets skipped).

### No duplicate work
Upstream's 0.5.18 unread/notification/archive work does **not** duplicate anything of ours. Their unread change is infrastructure (localStorage → SQLite, JS → Rust); our sidebar rollup lives in the render layer they did not touch, and `@channel`/`@here` does not exist upstream at all (`git grep "mention-scope"` at 0.5.18 returns nothing; their `broadcast` tag means "thread reply echoed to the main timeline", a different concept). Their notification work is TS display-name/click-target logic; `commands/notifications.rs` is **unchanged** between 0.5.17 and 0.5.18 and has no AUMID/WinRT code. Their `#6252` link metadata handles `buzz://` *internal* links; ours handles external http(s) URLs in a Files tab. Keep everything.
## What upstream actually has (checked against `upstream/main` @ `#6401`, 2026-08-21)
- **Upstream has NO Files tab.** `shared/api/channelFiles.ts` and `features/channels/ui/FilesPanel.tsx` do not exist in `block/buzz` at all. Neither does `fileVersionChains.mjs` or `sectionUnreadRollup.mjs`. Check this before writing another upstream-candidates entry — it was assumed twice that these were upstream features we had extended.
- **Consequence for upstreaming**: "links in the Files tab" is **not** a standalone PR. It sits on top of a stack that is entirely ours — Files tab → version chains → links. Offering it means offering the whole stack, or rewriting links to stand alone without versioning.
- **Drive routing is independent of that stack.** Every frontend file it touches exists upstream (`imetaMediaMarkdown.ts`, `useMediaUpload.ts`, `backgroundMediaUploadStore.ts`, `videoFileType.ts`, `tauriMedia.ts`). **But `google_meet.rs` does not** — upstream has no Google account connection at all. So a Drive PR necessarily carries the whole OAuth/PKCE/keyring module with it, which makes it far larger than the Drive code alone.
- **Upstream's `lib.rs` is 932 lines**, comfortably under the ceiling. The `command_registry.rs` extraction was needed *here* and is **not** needed there — leave it out of any PR, or it reads as gratuitous restructuring of their entry point.
- Files that DO exist upstream and that our Tier 1 fixes touch: `app/useChannelActivityProjection.ts`, `features/messages/ui/useMentionSendFlow.helpers.ts`, `features/sidebar/ui/SidebarSection.tsx`.
- **Pinned messages are OURS, not upstream's.** `features/channels/ui/PinnedMessagesBar.tsx` does not exist at `desktop-v0.5.18`, and upstream's `ChannelPane.tsx` does not mount it. `docs/buzz-feature-inventory.md` lists pinned messages under "Messaging core" without saying so — that document describes *the product*, not *what we added*, and the two have been conflated repeatedly.
- **The whole in-app file preview stack is ours**: `shared/ui/filePreview/` (8 components) plus `commands/pptx_conversion.rs`. `FilesPanel.tsx` depends on it, which is why any upstream Files-tab PR should drop the preview and download on click instead.
- **Cheapest way to answer "is this ours?"**: `git cat-file -e desktop-v0.5.18:<path> && echo upstream || echo ours`. Run it before writing another upstream-candidates entry.

## Google Drive routing (`0.5.17-1`)
- **What routes**: any file over `DRIVE_UPLOAD_THRESHOLD_BYTES` (5 MB), plus **all** video and audio at any size, plus (from `0.5.18-1`) **native executables** — PE, ELF, Mach-O. Those genuinely failed to upload before.
- **`driveUploadRouting.mjs`'s blocked list is deliberately NOT a copy of `BLOCKED_FILE_MIME_TYPES`** (`crates/buzz-media/src/validation.rs`), and a future reader **will** be tempted to "fix" that. Don't. The relay reaches its list only after sniffing bytes with `infer`, which has no matcher for most of the entries: **SVG sniffs as `text/xml`, JS and XHTML have no magic bytes, `.msi` sniffs as `x-ole-storage`, `.apk` as `zip`, `.dmg` not at all.** All six upload fine today. Only PE/ELF/Mach-O are reachable.
- **This was caught by audit, one step before shipping.** The first draft routed all 13 types to Drive on the assumption the Rust list was the truth. Because the Drive path *refuses rather than falling back*, that would have converted a working SVG upload into a hard error for every user without a Google account connected — and shipped a changelog telling users SVG "can now be shared" when it always could. The lesson: **`BLOCKED_FILE_MIME_TYPES` is a deny list, not a rejection list.** Check reachability before trusting it.
- **`text/html` is not blocked either**, for a separate and explicitly documented reason — the relay serves it as an inert download. Large HTML routes to Drive on the size rule only.
- **Honest framing of the security argument**: `validation.rs` gives two reasons for blocking executables — stored-XSS on header regression, and malware distribution. The Drive path genuinely removes the first. It **relocates** the second rather than removing it. Security-neutral unblocking, not a security improvement.
- **Known gap**: an extensionless ELF/Mach-O binary is invisible to the renderer and still fails. Pre-existing, not worth sniffing magic bytes client-side to fix.
- **Known, accepted, not fixed** (from the `0.5.18-1` audit): the Drive upload has no size cap of its own and holds the file twice in memory; a 308 on the *final* chunk abandons the session instead of resuming from Drive's `Range` header; and Google's raw error bodies are interpolated into user-facing messages.
- **Unrelated pre-existing issue worth a ticket**: `shared/ui/filePreview/XlsxPreview.tsx` uses `dangerouslySetInnerHTML` **without** `sanitizeDocumentHtml`, unlike its Docx and Pptx siblings. SheetJS escapes cell text, but `sheet_to_html` also emits `<a href>` from workbook-controlled hyperlink metadata. Reachable only via a relay-uploaded `.xlsx`.
- **The native file picker bypasses routing entirely** (`useMediaUpload.ts`, the non-`queueUntilSend` paperclip branch calls `pickAndUploadMedia`, a Rust-side relay upload). Files chosen that way ignore *all* the Drive rules, including the 5 MB and video/audio ones from `0.5.17-1`. Benign but inconsistent; worth closing. Rules are pure and tested in `features/messages/lib/driveUploadRouting.mjs`. Video detection is *not* reimplemented there — `isVideoFile` is passed in from `videoFileType.ts`, which already owns that decision for the deferred-upload split.
- **One seam, deliberately.** `routedMediaUpload.ts` is signature-compatible with `uploadMediaFile`, so `useMediaUpload.ts` and `backgroundMediaUploadStore.ts` each changed exactly one import line and nothing else. That was not only tidiness: `useMediaUpload.ts` is 1026 lines, over the ratchet ceiling, and **cannot grow by a single line**. Same for `MessageComposer.tsx` (1111) and `ComposerAttachments.tsx` (1012). Any future composer work has to extract first.
- **A Drive attachment is `external`, and that flag does two things** (`imetaMediaMarkdown.ts`): `buildImetaTags` skips it, and `formatImetaMediaLine` forces the `[filename](url)` link branch. Both are load-bearing. Without the first, the message asserts an `imeta` with an empty sha256 and a Drive viewer URL, which the Files tab and the download path both act on. Without the second, a `video/*` attachment renders `<video src="https://drive.google.com/…/view">` — a permanently broken player.
- **The link label is the naming mechanism.** Buzz writes the filename as the markdown label, and `channelLinkEntries.mjs` reads it back. That single fact removed the Drive-title API call the spec originally called for. If you ever change how Drive uploads are posted, the Files tab silently starts saying "Google Drive file" instead of the filename.
- **No `permissions.create` call.** The Workspace default for k2alpha.ai is "anyone in k2alpha.ai with the link", set 2026-08-21, so the call would never change an outcome. `google_meet/drive.rs`'s module comment records where it goes if that default is ever narrowed.
- **No "upload to Drive?" prompt.** Direct sharing of these files is not permitted, so the only alternative is not sending it. A dialog with one real outcome is a confirmation, not a choice.
- **Meet-connected ≠ Drive-connected.** Accounts connected before this shipped hold a refresh token minted without `drive.file`. It refreshes fine and then 403s. `google_access_token` returns the *granted* scope so this is caught before the upload starts and answered with "disconnect and reconnect under Settings → Voice".
- **Resumable upload gotchas**, both commented at the code: Drive answers intermediate chunks with **308**, which `reqwest` reports as an ordinary failure — 308 is the success case for every chunk but the last. And a zero-byte file still needs one `Content-Range: bytes */0` request or the session never completes. Chunks must be multiples of 256 KiB; we use 8 MiB.
- **Drive routing is OFF under e2e** unless a spec sets `__BUZZ_E2E__.mock.driveUploads`, mirroring `deferredComposerUploads`. Several specs upload a 16 MB file or a video specifically to assert the relay path. **Consequence: the Drive path has no end-to-end coverage at all.**
- **`lib.rs` was briefly extracted to make room, and un-extracted one day later.** Registering two commands needed two lines and `lib.rs` was at 1013, so the 320-line `generate_handler!` list moved to a `command_registry.rs` macro. Upstream then shipped 0.5.18 with `lib.rs` at 932 *and* 13 new commands, which would have landed inside a 338-line conflict where losing them fails silently at runtime. The extraction was reverted during that catch-up and `command_registry.rs` deleted. **Do not reach for this again** — if `lib.rs` is over the ceiling, extract something upstream does not also edit every release.
- **Not built**: resume-after-failure. A dropped connection restarts the upload. The session URI needed to continue is already in hand.

## Inbox (three lanes, settled in `0.5.14-3`)
- **The Inbox shows only what is addressed to you**: mentions, threads you are part of, and workflow approvals. Plain channel/DM activity is the sidebar's job, not the Inbox's.
- **`0.5.14-2` briefly did the opposite** — one row per channel or DM with an unread count, derived from `topLevelUnreadChannelIds ∪ unreadThreadChannelIds`. It was reverted a day later after being used: an Inbox that lists everything is the sidebar with extra steps. `channelUnreadRows.mjs` and `useChannelUnreadRows.ts` were deleted, and the `unreadChannelCounts`/`latestUnreadActivityByChannelId`/`mutedChannelIds` additions to `AppShellContext` came back out. **Do not re-add channel rows without revisiting that decision** — it has been tried.
- **Only two mechanisms sit under the three lanes.** Mentions and approvals come from the relay via `get_feed`; threads are assembled client-side in `useChannelActivityProjection` from observed events. `get_feed` returns `activity: Vec::new()` and `agent_activity: Vec::new()` — those two buckets are hardcoded empty (`commands/messages.rs`), so the frontend's `types: "...,activity,agent_activity"` request is vestigial. The Inbox's `agent_activity` *filter* still works because it is reimplemented client-side by sender pubkey (`inboxViewHelpers.ts`), not from the bucket it shares a name with.
- **Mentions supersede threads.** A reply that @-mentions you classifies as a mention only, never also a thread — one guard at the top of `matchesInboxFilter`'s thread branch. Without it a single item appeared under both filters and neither lane described what was actually waiting. Mentions (kinds `9, 40002, 1, 45001, 45003` + git kinds) and approvals (kinds `46010–46012`) are already disjoint by kind, so no guard is needed there.
- **Thread membership** is `!muted && (followed || participated || authored || mentioned)` (`AppShell.tsx`). Participated = you replied; authored = you wrote the root.
- **Threads are the only lane with two pieces of state** — that membership rule *and* their own read marker (`thread:<rootId>`), separate from the channel's. That is why every Inbox bug so far has been in this lane.
- **`resolveThreadActivityItemReadAt` exists because of one of them.** `resolveChannelActivityFeedItemReadAt` consults only `msg:<id>` and the channel marker, so reading a thread — which advances `thread:<rootId>` and nothing else — left every reply looking unread forever. The thread frontier must stay folded in; `useChannelActivityProjection.test.mjs` guards it with a case where only the thread marker moved.
- **The local unread override is not a bug.** `feedItemState.unreadSet` (localStorage `buzz-home-feed-unread.v1:<pubkey>`) short-circuits the read check, and is only ever written by the explicit "Mark as unread" action — persisting is the point. Known narrow gap: marking an item unread and then reading its thread *in the channel* does not undo the override, because only the Inbox's own read path calls `undoUnread`. Closing it means reaching into `useChannelUnreadState`; judged not worth the blast radius.
- **Drafts, Reminders and Project filters are deliberately untouched** by the three-lane change. They are features with their own panels and storage, not classifications of incoming activity, and removing their filters would orphan them rather than tidy anything.
- **Unread-only is the default** (`HomeView.tsx`), session-only rather than persisted, so a launch always starts focused. A read row survives under that filter only while it is the *selected* conversation — the list must not yank out what you are reading — and it carries a "Viewing" pill, because without one it is indistinguishable from a row that refuses to clear. That exact confusion was reported before the pill existed.

## Sidebar sections
- **Channel sections were already built upstream** — creation, icon, ordering, per-channel assignment, and cross-device sync via a kind-30078 replaceable event (`channelSectionsStorage.ts`, `channelSectionsSync.ts`, `useChannelSections.ts`). Twice now this has been re-specced from scratch before someone checked. **Look in `features/sidebar/lib/` before designing anything sidebar-shaped.**
- **Collapsed sections roll their contents up onto the header** (`sectionUnreadRollup.mjs`, 13 tests): a count when the section holds something high-priority, a dot for ordinary activity, nothing when quiet. Only while collapsed. Reuses the row-level badge classes and the same unread sets the rows read, so a header cannot contradict the rows it hides — which is the entire point.

## Global mentions (`@channel` / `@here`) — working as of `0.5.14-5`
- **Detected from the message text, not the autocomplete.** `detectMentionScope` matches the literal words at send time. `useMentions` is a ~990-line hook carrying debouncing, personas and teams; threading two synthetic entries through it is a far larger change than this needed to start working. Autocomplete remains a clean follow-up.
- **The matching guards both edges** with `(?![\w-])` rather than `\b`. `\b` treats a hyphen as a boundary, so `docs/@here-ish` and `@channel-ops` would have paged everyone. The expensive false positive here is silently notifying forty people; tests cover the lookalikes.
- **`@here` and the catch-up path.** Live events classify with the client running, so the user is "here". The *catch-up* branch in `useUnreadChannels` passes `presence: "offline"` — those events arrived while the client was off, so an `@here` the user missed must not accumulate as urgent. That is the entire distinction from `@channel`, which does persist for people who were away.
- **No permission gate is wired.** `canUseMentionScope` is built and tested (admins-only above 32 members, per WhatsApp) but unused, so anyone can `@channel` a 40-person room. Deliberate, pending real usage. Note any such gate is a *client-side convention* — events are signed client-side and no server refuses to publish, so a modified client could send one regardless. Same cooperative model as deletion; do not describe it to users as enforcement.
- **One marker tag, not N `p` tags.** Writing the audience into the event would bloat it, leak the member list, and freeze the audience at send time. The marker is resolved against *current* membership, so someone who joins later still sees the message as addressed to them.
- **Every caller-supplied tag group in `events.rs` is validated against its own prefix** (`imeta_tags`, `emoji_tags`, `mention_reference_tags`, `supersedes_tags`) explicitly to stop forged `h`/`e`/`p` tags. There is therefore **no generic passthrough** — a new tag needs its own validated parameter. `mention_scope_tag` follows that pattern.
- **A scoped message's audience is decided by the scope alone.** `shouldNotifyForEvent` returns the scope decision directly rather than falling through to the generic rules; an earlier version fell through and notified the author of their own broadcast.
- **`canUseMentionScope` is built and tested but not wired.** Do not mistake it for an active permission gate. Note also that any such gate is a *client-side convention* — events are signed client-side and no server refuses to publish, so a modified client could send one anyway. Same cooperative model as deletion; do not describe it to users as enforcement.
- **Still to build:** autocomplete entries, timeline rendering that marks a broadcast visibly, the permission gate, and the per-user opt-out from `@channel` piercing mute.

## Help agent (`#help` channel)
- **Delivered as a managed agent in a channel**, not a Settings panel. It answers only when tagged.
- **The system prompt lives in the agent's "Describe what this agent should do" field in the Agents UI — nowhere else.** `BUZZ_AGENT_SYSTEM_PROMPT_FILE` does *not* work for managed agents: they run through buzz-acp, which delivers the prompt via `session_new_full`. `docs/help-agent-prompt.md` is the maintained *source text* that must be pasted into that field; editing the file alone changes nothing about the running bot.
- **Gemini via the OpenAI-compatibility shim** — `buzz-agent` has no native Gemini provider. Working config: harness `buzz-agent`, provider `openai-compat` (labelled "OpenAI-compatible" in the UI), model `gemini-3.5-flash-lite` entered as a custom entry, `OPENAI_COMPAT_BASE_URL = https://generativelanguage.googleapis.com/v1beta/openai/`, `OPENAI_COMPAT_API = chat`.
- **Undocumented-question backlog**: the prompt requires the exact phrase `Not documented yet` on every decline, so searching the channel for that string collects every question the docs could not answer. Paraphrasing it makes the gap invisible — the fixed wording is load-bearing, not style.
- **Maintenance**: `docs/help-agent-prompt.md` is the agent's entire knowledge of the product. A release that changes user-facing behaviour must update it *and* re-paste it, or the bot answers confidently and wrongly.

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
- **Repo secrets added** (2026-08-15), so release builds from `0.5.14-1` onward bake in real credentials. Earlier builds — including `0.5.14-0` and everything before it — were built without them and will report "not configured for this build" regardless of Google-side setup.
- **Not yet done**: a real end-to-end test — connect an account, start a meet, confirm the link posts and opens correctly. Worth doing on the first installed build that has the secrets baked in, since nothing before `0.5.14-1` could have worked. Also unverified: whether the OAuth client is Desktop-app type (the loopback flow requires it; a Web-type client compiles and installs fine, then fails at the consent step).
