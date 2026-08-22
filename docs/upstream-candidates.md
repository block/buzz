# Upstream candidates

Changes in this fork that we believe belong in [`block/buzz`](https://github.com/block/buzz),
ranked by how cleanly they would land. Maintained so the divergence stays
deliberate rather than accidental.

Nothing here has been submitted yet. If you are an upstream maintainer reading
this, take any of it.

---

## Tier 1 — bug fixes, small and self-contained

These fix behaviour that is wrong upstream today. Each is a few files, already
tested, and carries no product opinion.

### Thread activity never clears from the Inbox

`resolveChannelActivityFeedItemReadAt` consults only the per-message marker
(`msg:<id>`) and the channel marker. Reading a thread advances
`thread:<rootId>` — a key it never looks at. The consequence is that marking a
thread read leaves every one of its replies looking unread indefinitely, unless
each reply happened to be marked individually.

The fix folds the thread frontier in via the existing `getThreadReadAt`, which
already computes the right value fifty lines above in the same file and simply
was not used on this path.

*Files:* `app/useChannelActivityProjection.ts` (+ tests).
*Risk:* low. Regression test included — a case where only the thread marker
moved, which returned `null` before the fix.

### Tauri error messages are discarded, so failures read as "unknown error"

`getErrorMessage` (`useMentionSendFlow.helpers.ts:85`) only keeps a message when
the value is an `Error` instance:

```ts
return error instanceof Error && error.message ? error.message : fallback;
```

Tauri's `invoke` rejects with a **plain string**, not an `Error`. So every
error raised from Rust fails the `instanceof` check and is replaced by the
caller's fallback — usually `"unknown error"`.

The user-visible cost is concrete. `find_ffmpeg` in `commands/media_transcode.rs`
raises a genuinely helpful message when video upload can't find ffmpeg —
including `brew install ffmpeg` — and none of it reaches the user. On macOS,
where ffmpeg is not preinstalled, the first person to share a video gets
`Upload failed: unknown error` and no path forward. It took reading the Rust
source to work out what was wrong.

The fix is to fall back to the string form before the literal, which
`useMediaUpload.ts:630` already does correctly with `String(err)` — so the
codebase contains both the right and the wrong handling of the same thing.

*Files:* `features/messages/ui/useMentionSendFlow.helpers.ts`.
*Risk:* very low. Strictly widens what is reported; no path loses information.

### Unread dot invisible on the active channel row

The row-level unread dot painted `bg-primary` unconditionally. The active row
paints `bg-sidebar-active`, which every theme defines as
`var(--sidebar-primary)` — the same hue. So on the channel you currently have
open, the dot drew primary-on-primary and effectively disappeared.

That is precisely the case it exists for: you are reading a channel when a reply
lands in a thread you do not have open, and the single signal you get is
invisible. Active rows now use the active row's foreground colour, matching how
the muted-bell icon in the same file already behaves. Also sized 8px → 10px; at
the end of a row with nothing adjacent for scale, 8px read as a rendering
artifact.

*Files:* `features/sidebar/ui/SidebarSection.tsx`.
*Risk:* very low, cosmetic.

---

## Tier 2 — features on top of upstream's own primitives

These extend things upstream already built, and should be uncontroversial in
shape even if the details get debated.

**Only the first entry actually qualifies.** Verified against `desktop-v0.5.18`
on 2026-08-21: upstream has channel sections, so the rollup genuinely extends
their work. It has **no Files tab** — `shared/api/channelFiles.ts` and
`features/channels/ui/FilesPanel.tsx` do not exist upstream at all — so the two
file-related entries below build on nothing of upstream's and are a much larger
proposition than their placement here implies. They are kept in this section
because they are still the things we would most like taken; read their own
notes for the real scope.

### Collapsed sections do not show what is inside them

Upstream has channel sections — creation, icons, ordering, per-channel
assignment, and cross-device sync via a kind-30078 replaceable event. But a
collapsed section renders its title and chevron and nothing else, so folding
channels away hides their activity completely. That makes sections actively
risky on a large sidebar: you tidy up and stop seeing things.

This fork rolls the section's contents up onto the header while collapsed: a
count when it holds something high-priority (an unread DM, a message tagging
you, a broadcast), a dot for ordinary activity, nothing when quiet. It reuses
the existing badge components and the same unread sets the rows read, so a
header cannot contradict the rows it is hiding.

*Files:* `features/sidebar/lib/sectionUnreadRollup.mjs` (+ `.d.mts`, 13 tests),
`features/sidebar/ui/SidebarSection.tsx`, one addition to `AppShellContext`.
*Risk:* low. The rollup is pure and fully tested.

### Links are invisible to the Files tab

**Scope warning:** upstream has no Files tab. This cannot be offered on its own
— it is the top of a stack that is entirely ours (Files tab → version chains →
links), and any PR must either carry that stack or rewrite links to stand alone
without versioning. The "additive" risk note below is true only relative to our
own code.

A channel's Files tab lists uploads and nothing else, but a lot of what a team
actually works from is a link — a Google Doc, a dashboard, a file too large to
upload. Those are scattered through the timeline and findable only by scrolling.

This fork makes any http(s) URL in a message a first-class entry in the same
list: one row per unique URL, dated at its earliest appearance, removed when its
message is deleted.

**The reason it is small is worth stating**, because it is a property of
upstream's own design rather than anything this fork invented. The supersedes
tag references an **event**, not a file, so nothing in the version-chain
machinery ever cared whether the entry behind an event id was an upload. Links
therefore get version chains, tombstone handling and the composer's supersedes
picker with no new tag, no new event kind and no relay change. A Google Doc can
supersede an uploaded PDF, and vice versa.

Naming is the only hard part, since the document title is not in the event and
link previews are fetched at render time rather than stored. The chain is: the
sender's own markdown label (`[Q3 Budget](https://…)`), then a recognised Google
surface (`Google Doc`, `Google Drive file`), then the last non-opaque path
segment, then the host. Opaque ids are skipped rather than shown — a bare Drive
id in a file list is worse than the hostname.

Two rules that look arbitrary and are not, both found by writing the tests:

- **A URL already present as an upload's own `url` never becomes a link entry.**
  The markdown renderer embeds an attachment's URL in the message body, so
  without this every upload also produces a duplicate link row beside itself.
- **A supersedes tag reaches a link only when its message carries exactly one
  link and no attachment.** Otherwise two links in one message both claim the
  same predecessor, or a link takes a tag belonging to the file beside it.

*Files:* `shared/lib/channelLinkEntries.mjs` (+ `.d.mts`, 30 tests), a `kind`
discriminator on `ChannelFileEntry`, and link rows in `FilesPanel.tsx`.
*Risk:* low. The extraction and naming are pure and fully tested; the merge into
`listChannelFiles` is additive.

### File versioning

**Scope warning:** same as above — the Files tab it renders into is ours, not
upstream's, and so is the in-app file preview (`shared/ui/filePreview/`, 8
components plus `commands/pptx_conversion.rs`) that `FilesPanel` opens. A PR
should drop the preview and download on click instead, or it becomes three
features at once.

The largest piece, and the one we would most like upstream to take, because
"which version of this is current" is a problem every team hits and no amount
of channel discipline solves.

The design is shaped by event immutability: a message cannot know it has been
superseded, because that fact lives in a later message. So version chains are
derived at read time from the channel's file list rather than stored, and the
graph walking tolerates cycles, forks, self-supersedes and missing parents —
the input arrives over a relay and is not assumed well-formed.

Includes fuzzy filename matching (`report-v2.pdf`, `report (1).pdf`,
`deck FINAL.pptx`), a bounded candidate window, and refusal to pre-select on a
tie.

*Files:* `features/messages/lib/fileVersionChains.mjs`,
`supersedesRanking.mjs`, `shared/api/supersedesTags.ts`,
`shared/context/FileVersionContext.tsx`, `shared/ui/FileVersionBadge.tsx`,
plus `channelFiles.ts` and `FilesPanel.tsx` changes. 22 tests.
*Risk:* moderate — it is a real feature with UI surface.

**One hard-won lesson worth carrying over.** An earlier version published the
version link as an **empty-content kind:9 event**. Kind 9 is the ordinary
channel-message kind, and it had to be a timeline kind because `listChannelFiles`
can only discover events the relay returns for `TIMELINE_KINDS`. So making the
link *discoverable* also made it *renderable*, and every version tag posted a
blank message to the channel under the tagger's name. The link is now a tag on
the file's own message, set only at upload time. Do not reintroduce a separate
link event without solving that.

---

## Tier 3 — probably too opinionated to upstream

Recorded for completeness. These encode product decisions specific to how one
team works, and upstream may reasonably disagree.

- **Inbox reduced to three lanes** (mentions, threads, approvals), with
  mentions superseding threads and ordinary channel activity excluded. We
  reached this by shipping the opposite first and reverting it — see
  `CONTEXT.md`. Defensible, but a genuine product opinion.
- **Unread-only by default** in the Inbox.
- **Google Meet integration.** Useful, but it presumes Google accounts and
  build-time OAuth credentials.
- **Routing large files, video and audio to the sender's Google Drive.** See
  the section below — offered on its own terms rather than filed away.
- **Upstream release history in Settings → Updates.** Only meaningful for a
  fork.
- **The single-job Windows release pipeline.** Deliberately not upstream's
  multi-platform workflow, which hardcodes publishing to `block/buzz`.

---

## Offered as-is — Google Drive routing

Filed separately because it does not fit the tiers above. It is not a bug fix,
it is not built on an upstream primitive, and it is more opinionated than Tier 3
usually implies. It is also the single most useful thing in this fork for the
team that runs it, which is why it is here rather than kept private.

**The problem is general.** Relay uploads of large media are fragile: video
requires ffmpeg on the sender's machine (not preinstalled on macOS), transcodes
locally, then lands on a relay whose size caps, retention and version the client
does not control. On a hosted community that is someone else's storage budget.
The failure mode we hit was an mp4 reaching 100% and then failing with "unknown
error" — which is also how the `getErrorMessage` bug in Tier 1 was found.

**The solution is specific.** Files over 5 MB, and all video and audio at any
size, upload to a "Buzz uploads" folder in the sender's own Drive on the
narrow, non-restricted `drive.file` scope, and post as a labelled link. It
assumes a Google Workspace domain, an admin-set "anyone in the domain with the
link" sharing default, and build-time OAuth credentials.
`docs/google-drive-integration-spec.md` states every assumption in a table at
the top, and closes with a "Generalising this" section listing exactly what
would have to change to widen it — the load-bearing item being explicit
`permissions.create` calls instead of relying on that default, which is the one
assumption whose absence fails *silently*.

**Two parts of it are general even if the whole is not:**

- **The `external` attachment flag** (`imetaMediaMarkdown.ts`, ~15 lines) means
  "the bytes live outside the relay". Such an attachment emits no `imeta` tag —
  there is no blob and no sha256 to honestly assert — and always renders as a
  plain `[filename](url)` link, never inline, because a `<video>` pointed at a
  viewer page is a permanently broken player. That is the primitive any
  storage-elsewhere backend needs.
- **The single seam.** `routedMediaUpload.ts` is signature-compatible with
  `uploadMediaFile`, so the composer and the deferred-upload queue each changed
  one import and nothing else. S3, Dropbox or a self-hosted bucket would slot in
  beside Drive without touching composer code.

*Files:* `src-tauri/src/google_meet/drive.rs`, scope and token changes in
`google_meet.rs`, `features/messages/lib/{driveUploadRouting.mjs,
routedMediaUpload.ts}`, `shared/api/tauriDrive.ts`, the `external` flag in
`imetaMediaMarkdown.ts`, and one-line import swaps in `useMediaUpload.ts` and
`backgroundMediaUploadStore.ts`.
*Risk:* moderate. Routing rules are pure and tested; the upload path is
deliberately excluded from e2e (no Google account in CI) and is verified by use
rather than by CI.

---

## Notes for whoever submits these

- Tier 1 should go as two separate small PRs. They are unrelated and mixing
  them makes both harder to review.
- Every item above already has tests in the `.mjs` + `.d.mts` pattern upstream
  uses, so they should run unmodified.
- The file-versioning work assumes the relay's kind-40099 tombstones for
  deletion handling. If upstream's relay differs, that part needs revisiting.
