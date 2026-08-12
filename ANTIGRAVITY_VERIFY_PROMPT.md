Verify and clean up a batch of changes made to this Buzz fork (Tauri desktop app + Rust/Nostr relay) across several features. No compiler or typechecker was available while these changes were made, so nothing has been built or tested yet — your job is to get this to a genuinely verified, deployable state. Work through the sections in order; each one has its own pass/fail bar.

## 0. Manual file deletion (do this first)

These four files were emptied to zero bytes and fully de-registered from the build (no module/route/handler references them anymore), but were never actually deleted from disk. Delete them for real:

```
crates/buzz-relay/src/api/files.rs
crates/buzz-media/src/drive.rs
desktop/src-tauri/src/commands/channel_files.rs
desktop/src/shared/api/drivePreview.ts
```

After deleting, grep the repo for `files::`, `mod files`, `drivePreview`, `channel_files` to confirm nothing still references these paths.

## 1. Rust: compile, lint, test

```
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo fmt --check
```

Fix whatever comes up. Pay particular attention to these areas, since they had the most invasive changes:

- `desktop/src-tauri/src/events.rs` — added `supersedes_tags()`, `check_event_id()` validators, and a `build_supersedes_link()` builder; a `supersedes_tags` param was threaded through `build_message`/`build_message_with_client_tags` at all 5 call sites (including `egress_guard_tests.rs` and `huddle/pipeline.rs`).
- `desktop/src-tauri/src/commands/messages.rs` — `send_channel_message` gained a `supersedes_tags` param; a new command `link_channel_file_versions` was added.
- `desktop/src-tauri/src/lib.rs` — confirm `link_channel_file_versions` is registered in `tauri::generate_handler![...]` and that `list_channel_files`/`get_drive_preview` are NOT (they were removed along with `channel_files.rs`).
- `desktop/src-tauri/src/commands/mod.rs` — confirm `mod channel_files;`/`pub use channel_files::*;` were removed cleanly.
- `crates/buzz-relay/src/router.rs`, `crates/buzz-relay/src/api/mod.rs` — confirm no dangling route registrations or `pub mod files;` remain.
- `crates/buzz-relay/src/handlers/imeta.rs`, `crates/buzz-relay/src/handlers/ingest.rs` — confirm `validate_supersedes_tag` and its call site are fully gone (this was deliberately deleted as dead code, don't resurrect it).
- `crates/buzz-media/src/lib.rs`, `crates/buzz-media/src/config.rs`, `crates/buzz-media/src/storage.rs`, `crates/buzz-media/Cargo.toml` — confirm `drive` module, `GoogleDriveConfig`, `DrivePreviewMeta`/sidecar functions, and the `reqwest` dependency (if genuinely unused elsewhere — re-check this, don't just trust it) were cleanly removed.
- `crates/buzz-cli/src/lib.rs` — confirm the `GoogleDriveAuth` subcommand and `run_google_drive_auth` are gone.
- `crates/buzz-agent/src/auth.rs` — confirm `get_refresh_token` was removed only if it truly has no other callers (re-verify this, the removal was done via grep by an agent without compiler feedback).

## 2. Desktop frontend: typecheck, lint, build, test

```
cd desktop
npm run typecheck   # or: npx tsc --noEmit — use whatever this repo's package.json actually defines
npm run lint
npm test
npm run build
```

Areas with the most invasive changes — check these first if anything fails:

- `desktop/src/shared/api/channelFiles.ts` — `listChannelFiles` was rewritten from scratch to page through `getChannelMessagesBefore` and parse `imeta`/`supersedes` tags client-side (replacing a dead custom-relay-endpoint call). Also added `supersedesLinkDeclaration()` parsing and `linkChannelFileVersions()`.
- `desktop/src/features/channels/ui/FilesPanel.tsx` — restructured `FileRow` (button → div wrapper) to add a hover "link to another file" action; uses `useQueryClient` to invalidate `["channel-files", channelId]` after a successful link.
- `desktop/src/features/messages/ui/FileVersionPicker.tsx` — new shared component, a searchable popover reusing existing `Popover`/`PopoverContent` primitives.
- `desktop/src/features/messages/ui/ComposerAttachments.tsx` — gained a `channelId` prop, a `SupersedesToggleRow` (auto-suggested match, opt-in toggle) and a `ManualSupersedesPickerRow` ("Link to a different file…"). Confirm other importers of this component (there should be at least one using only `DropZoneOverlay`) still compile with the new optional prop.
- `desktop/src/features/messages/ui/MessageComposer.tsx` — filename-match detection effect (normalized filename match against `listChannelFiles`, excluding same-sha256 and already-superseded candidates, singular-candidate only); passes `channelId` into `ComposerAttachments`.
- `desktop/src/features/messages/lib/useMediaUpload.ts` — added `supersedesByUrl` map + `setAttachmentSupersedesEventId`, cleared on new draft epoch and on attachment removal.
- `desktop/src/features/messages/lib/imetaMediaMarkdown.ts` — `buildSupersedesTags()`, extended `splitOutgoingTags()` to bucket `["e", id, "", "supersedes"]` tags separately from imeta/emoji/mention tags. Check `imetaMediaMarkdown.test.mjs` passes.
- `desktop/src/features/messages/ui/useMentionSendFlow.ts`, `desktop/src/features/messages/hooks.ts` (`useSendMessageMutation`) — thread `supersedesTags` through to `sendChannelMessage`.
- `desktop/src/shared/api/tauri.ts` — `sendChannelMessage()` gained a `supersedesTags` param; confirm every other call site of `sendChannelMessage` still compiles.
- `desktop/src/shared/ui/filePreview/FilePreviewModal.tsx` — the Google Drive preview toggle/iframe/props (`channelId`, `sha256`) were removed. Confirm `FilesPanel.tsx` (the only other consumer) doesn't still pass those removed props.
- `desktop/src/shared/ui/filePreview/PptxPreview.tsx` — new client-side `.pptx` renderer via `@jvmr/pptx-to-html` + DOMPurify sanitization (`desktop/src/shared/lib/sanitizeHtml.ts`). Confirm `package.json` actually has `@jvmr/pptx-to-html`, `dompurify`, `@types/dompurify` installed (`npm install` if `node_modules` is stale).
- `desktop/src/shared/lib/devBuildLabel.ts` — `DEV_BUILD_LABEL` should read `"k2v2"`. `desktop/src/features/settings/ui/SettingsView.tsx` should render `{appVersion}{DEV_BUILD_LABEL ? \`-${DEV_BUILD_LABEL}\` : ""}` (no extra space/dash before the label beyond the single hyphen).
- `desktop/src-tauri/tauri.conf.json` — confirm the CSP's `frame-src` was cleanly reverted to `'self';` (no leftover `https://drive.google.com`, no broken JSON syntax from the edit).

## 3. Manual smoke test against the real k2alpha community (BuilderLab-hosted)

This is the part that actually matters most — none of the above proves the app works against the user's real, hosted-by-Block relay, since it wasn't compiled at all until now. Once it builds clean:

1. Open a channel in k2alpha, attach a `.pptx` file, send it, and confirm it previews in-app (slides render, no relay/network errors).
2. Open the Files tab for that channel (new folder-icon button near the channel header) and confirm it lists files without erroring — this is the part that specifically must NOT depend on any custom relay endpoint (the old `GET /api/channels/{id}/files` is gone; it now pages through the same `/query` bridge every other read in the app already uses).
3. Upload a second file with the exact same filename as one already in the channel but different content. Confirm the composer shows a "New version of `<filename>`?" toggle. Leave it off, send, confirm the Files tab shows it as a normal untracked file (no badges).
4. Repeat, but turn the toggle on this time. Send, then check the Files tab: the older file should show an "Outdated" badge, the newer one a "New version" badge.
5. In the composer, attach an unrelated file and use "Link to a different file…" to manually pick an existing file as the one it supersedes. Confirm the same badges appear correctly after sending.
6. In the Files tab, hover a file row that isn't already outdated, click the link icon, pick a different file from the picker, and confirm the badges update immediately (query should auto-invalidate/refetch — no manual reload needed).
7. Confirm nothing related to Google Drive is visible anywhere (no "View in Google Drive" button in the file preview modal, no console errors referencing it).
8. Check Settings shows the version string as `<version>-k2v2`.

Report back: what failed at each stage (compile/lint/test/build), what you fixed, and the result of the manual smoke test — pass/fail per step above, with screenshots or console output for anything that failed.
