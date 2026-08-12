Verify and fix two batches of changes, none of which have been compiled or typechecked yet: (A) message pinning, message forwarding with a Ctrl+Click multi-select redesign, and a first-run "what's new" splash screen, and (B) a sidebar unread-legibility fix. Get all of it to a genuinely verified state, then smoke-test against my real k2alpha community (BuilderLab-hosted).

## 1. Rust: compile, lint, format, test

```
cargo check --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml
```

Focus areas:

- `desktop/src-tauri/src/commands/pinned_messages.rs` (new) — `get_pinned_messages`/`set_pinned_messages` commands. Confirm they use `Kind::Custom(40004)` only (NOT 40100/Canvas, NOT 10001/NIP-51 pin list), and that `set_pinned_messages` rejects more than 3 ids server-side.
- `desktop/src-tauri/src/events.rs` — `build_set_pinned_messages` addition.
- `desktop/src-tauri/src/commands/mod.rs`, `desktop/src-tauri/src/lib.rs` — confirm both new commands are registered in `generate_handler![...]`.

## 2. Desktop frontend: typecheck, lint, build, test

```
cd desktop
npx pnpm typecheck
npx pnpm lint
npx pnpm test
npx pnpm build
```

### Batch A — pinning, forwarding, what's new splash

Read each file fully before assuming anything about its current shape, this batch went through an extra redesign pass:

- `desktop/src/features/channels/hooks.ts` — `MAX_PINNED_MESSAGES`, `usePinnedMessagesQuery`, `useSetPinnedMessagesMutation`.
- `desktop/src/features/channels/ui/PinnedMessagesBar.tsx`, `usePinnedMessagesActions.ts` — confirm: 0 pins renders nothing, exactly 1 pin renders a standalone banner, 2-3 pins collapse into one "N pinned messages" bucket (never multiple stacked banners), a 4th pin attempt is blocked with a toast (check both the client-side guard and that the server rejects it too).
- `desktop/src/features/messages/ui/MessageActionBar.tsx` — Pin/Unpin toggle in the "..." menu; Forward is a **direct toolbar icon** next to Reply (not in the dropdown) — confirm it renders correctly, uses a sensible lucide icon, and opens `ForwardMessageDialog` for the single message.
- `desktop/src/features/messages/ui/MessageSelectionContext.tsx` — selection is implicit: `active` is derived from `selected.size > 0`, there's no separate enter/exit mode anymore, just `toggle`/`clear`.
- `desktop/src/features/messages/ui/MessageRow.tsx` — **highest risk file in batch A.** A `handleRowClick` was added via `onClickCapture` on the row root: Ctrl/Cmd+click toggles that message into/out of the selection via capture-phase interception (`preventDefault`/`stopPropagation`) before any inner element's click (avatar, links, collapse controls) fires; a plain click falls through to normal behavior unchanged. Specifically verify: (a) Ctrl+clicking a link/avatar doesn't ALSO trigger its normal action, (b) plain clicks everywhere else in the row are unaffected, (c) the selection checkbox appears on every row once any message is selected, not just the one you Ctrl+clicked.
- `desktop/src/features/messages/ui/ForwardMessageDialog.tsx`, `desktop/src/features/messages/lib/forwardMessageContent.ts` — destination picker (channels + people) and content-bundling (blockquoted per-message content, chronological order, unioned `imeta` tags carried forward without re-upload). Not touched by the redesign pass, only how they're triggered changed.
- `desktop/src/features/messages/ui/MessageTimeline.tsx` — wraps children in `MessageSelectionProvider`.
- `desktop/src/features/whatsNew/` (new folder: `changelog.ts`, `whatsNewStorage.ts`, `useWhatsNewModal.ts`, `ui/WhatsNewModal.tsx`) — confirm the versioned changelog array only shows entries up to and including the current `DEV_BUILD_LABEL` (should be `"k2v4"` — check `desktop/src/shared/lib/devBuildLabel.ts`), persistence is local-only (`window.localStorage`, not published to the relay), and dismissal requires clicking "Got it" (no dismiss via Escape or outside-click).
- `desktop/src/app/AppShell.tsx` — `WhatsNewModal` mounted post-auth, excluded from the huddle companion window.

### Batch B — sidebar unread legibility fix

- `desktop/src/features/sidebar/ui/SidebarSection.tsx` (`ChannelMenuButton`) — unread text weight changed `font-bold` → `font-semibold`. The old opacity-based contrast (`opacity-80` on read rows, same `text-sidebar-foreground` color for both states) was replaced with an actual two-color system applied to the icon+label span (NOT the button root, which is overridden by a Buzz-theme CSS rule for non-active buttons): unread → `text-sidebar-foreground` (full strength), read → `text-muted-foreground`, fully-read-and-muted → `text-muted-foreground/60`. Verify this actually renders with visibly better contrast in both light and dark theme, AND in the "Buzz" custom theme specifically (check for a `data-buzz-sidebar` attribute/theme toggle in settings) since the report flagged a theme-specific CSS override (`[data-sidebar="menu-button"]:not([data-active="true"]) { color: var(--buzz-channel-fg) }`) as the reason the color had to go on the child span instead of the button itself — confirm that override doesn't ALSO clobber the new span-level color classes.
- Same file — the unread dot condition changed from `hasThreadUnread` only to `hasThreadUnread || (hasTopLevelUnread && channel.channelType !== "dm")`. Verify: channels with ordinary unread messages (no open thread) now show a small dot; DMs do NOT get this dot (they already have a separate numeric unread-count badge — confirm that still renders correctly and isn't duplicated); a channel with BOTH thread-unread and top-level-unread shows exactly ONE dot, not two; muted channels/rows still lay out correctly with the mute icon (`BellOff`) alongside the dot with no visual overlap/collision.
- Confirm `desktop/src/features/home/ui/InboxListPane.tsx` (Inbox) was NOT touched — it already used the correct pattern and didn't need this fix, so it should show zero diff.

## 3. Manual smoke test against real k2alpha

### Pinning
1. Pin a message in a channel. Confirm it shows as a standalone banner at top.
2. Pin a second message. Confirm both now show as one collapsed "2 pinned messages" bucket, not two banners.
3. Try pinning a 4th message with 3 already pinned. Confirm a clear toast telling you to unpin one first, and the pin is rejected.
4. Click "jump to message" from the pinned bar/bucket and confirm it scrolls to and highlights the right message.
5. Unpin a message and confirm the bar updates immediately.
6. Repeat pin behavior in a DM.

### Forwarding
1. Click the Forward icon (next to Reply) on a single message. Confirm the destination picker opens with channels and people, searchable/selectable.
2. Forward it to one channel and one person you don't already have a DM with. Confirm a DM gets created/opened, and the message appears correctly formatted (quoted, attributed) in both destinations.
3. Ctrl+click two or three different messages (including one with a file attachment) to select them. Confirm a floating "Forward (N)" bar appears, and Ctrl+clicking one again removes it.
4. Forward the multi-selection to a channel. Confirm it arrives as ONE combined message, all quoted in original chronological order, attachment viewable without re-upload.
5. Click "Cancel" and confirm selection clears and checkboxes disappear.
6. Confirm plain (non-Ctrl) clicks anywhere on a message row still behave normally — no accidental selection, no broken links/avatar clicks.

### What's New splash
1. Confirm it appears automatically on this first launch of the k2v4 build, showing v2/v3/v4 sections in order with the right bullets.
2. Click "Got it," relaunch, confirm it does NOT show again.
3. Confirm Escape and outside-click do NOT dismiss it — only "Got it" does.

### Sidebar unread legibility
1. Get an unread channel and an unread DM in the sidebar (send yourself/have someone send a message). Confirm the unread name is now clearly, crisply legible — noticeably more readable than before, not just technically bold.
2. Confirm the unread channel shows a small dot indicator; confirm the unread DM shows its numeric count badge (not a duplicate dot).
3. Read the message, confirm the row returns to the dimmer "read" color/weight once marked read.
4. If this app has a theme toggle (light/dark/"Buzz" custom theme), check unread legibility in each — flag anything that looks broken or low-contrast in any of them.
5. Open Inbox and confirm its unread styling looks unchanged from before (it wasn't touched).

Report back: pass/fail for every compile/lint/test/build step (what broke, what you fixed), and pass/fail for every manual smoke-test step above with specifics (screenshot or exact repro) for any failure.
