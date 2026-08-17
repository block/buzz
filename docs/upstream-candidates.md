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

### File versioning

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
- **Upstream release history in Settings → Updates.** Only meaningful for a
  fork.
- **The single-job Windows release pipeline.** Deliberately not upstream's
  multi-platform workflow, which hardcodes publishing to `block/buzz`.

---

## Notes for whoever submits these

- Tier 1 should go as two separate small PRs. They are unrelated and mixing
  them makes both harder to review.
- Every item above already has tests in the `.mjs` + `.d.mts` pattern upstream
  uses, so they should run unmodified.
- The file-versioning work assumes the relay's kind-40099 tombstones for
  deletion handling. If upstream's relay differs, that part needs revisiting.
