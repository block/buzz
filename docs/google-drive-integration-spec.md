# Google Drive integration — spec

Route large files, video and audio through each person's Google Drive instead
of the relay, and make shared links first-class citizens of the Files tab.

Status: **built.** Written 2026-08-18 against `0.5.17-0`, shipped whole in
`0.5.17-1` on 2026-08-21. The phasing this document describes is history, not a
plan — it is kept because the reasoning still explains why the pieces are
shaped the way they are.

---

## What this assumes — read this first

This is a **specific workflow**, not a general storage abstraction. It works
well, and it works well for a particular shape of team. Decide in thirty
seconds whether that is you:

| Assumption | Why it matters |
|---|---|
| Everyone signs in with a **Google Workspace** account on a shared domain | A personal Gmail account has no domain to share to |
| An admin has set Drive's general access default to **"anyone in \<domain\> with the link"** | The app sets **no** per-file permission; it relies entirely on this default |
| The build supplies **Google OAuth client credentials** at compile time | Unconfigured builds compile fine and report the feature unavailable |
| The team is content with **5 MB** as the threshold | It is a constant, not a setting |
| Drive is connected under **Settings → Voice**, alongside Google Meet | One Google connection serves both; you cannot have Drive without also granting Meet's scope |

**The sharp edge is row two.** If that default is not set, the upload succeeds,
the link posts, and nobody but the sender can open it — silently, with no error.
The app does not detect this.

Anyone wanting this outside those assumptions should read "Generalising this" at
the end.

---

## Why

Two problems, one answer.

**Video uploads are unreliable.** They require ffmpeg on the sender's machine
(not preinstalled on macOS), transcode locally, then hit a hosted relay whose
size limits and version we do not control. When they fail, the error is
swallowed and the user sees "unknown error". We can fix the error message —
see `upstream-candidates.md` — but not the underlying fragility.

**We are on a hosted community.** Block runs the relay and the storage. Upload
caps, retention and quota are their policy, not ours. Pushing large media
through it is building on someone else's budget.

Drive sidesteps both. The file never touches the relay; the message carries a
link.

---

## What is in scope

1. **Links become first-class entries in the Files tab**, alongside uploads.
2. **Files over 5 MB, and all video/audio regardless of size, go to Drive.**
   Auto-shared to the `k2alpha.ai` domain on upload.
3. **Version chains work across links and files interchangeably.**

Explicitly **not** in scope: browsing an existing Drive folder from inside
Buzz. See "Why not folder browsing" below.

---

## Phase 1 — links as file entries (no Google involved) — **built**

Shipped first. Useful on its own, needs no OAuth, and is the thing that makes
Phase 2 cheap.

Implemented in `shared/lib/channelLinkEntries.mjs` (+ `.d.mts`, 24 tests), with
the merge into `shared/api/channelFiles.ts` and link rows in
`features/channels/ui/FilesPanel.tsx`.

Two rules the build added that this spec did not anticipate, both discovered by
writing the tests:

- **A URL that is already an uploaded file's own URL never becomes a link
  entry.** The markdown renderer embeds an attachment's URL in the message
  body, so without this exclusion every upload also produced a duplicate link
  row beside itself.
- **A supersedes tag is attributed to a link only when its message carries
  exactly one link and no attachment.** Otherwise two links in one message
  would both claim the same predecessor, or a link would take a tag that
  belongs to the file beside it.

### The insight

`listChannelFiles` (`shared/api/channelFiles.ts`) builds entries from `imeta`
tags. The version link is `["e", "<older-event-id>", "", "supersedes"]` — it
points at an **event**, not at a file. So version chains already work on
anything that is an entry in that list.

Add a link entry type to the same list and **items 1 and 4 both fall out of one
change**. A Drive link can supersede an uploaded PDF, or another link, with no
new tag, no new event kind, no relay change.

### Shape

A link entry uses the existing `ChannelFileEntry` shape:

- `eventId` — the message carrying the link
- `url` — the link itself
- `filename` — the display name; see naming below
- `uploadedAt`, `uploaderPubkey` — from the event
- `supersedes` / `supersededBy` — unchanged, already event-based

### Naming, in priority order

**As built** — and simpler than specified, because the third idea below made
the first one unnecessary:

1. **The sender's own markdown label.** `[Q3 Budget.xlsx](https://…)` names the
   entry `Q3 Budget.xlsx`. People already write links this way, and the
   composer already emits this exact form for file attachments.
2. **A known Google surface** — `Google Doc`, `Google Sheet`, `Google Slides`,
   `Google Drive file`, `Google Drive folder`.
3. **The last meaningful path segment**, skipping opaque ids.
4. **The host.**

Never show a bare Drive ID. `1a2B3c...` in a file list is useless.

**The Drive API title lookup was specified and then not needed.** Because Buzz
controls how it posts its own Drive uploads, it writes the filename as the
markdown label at post time — so rule 1 already names every file Buzz uploaded,
with no network call, no OAuth dependency for reading, and no per-row latency
in the Files tab. The lookup would only ever have added value for a Drive link
someone pasted by hand without a label.

### Rules

- **One entry per unique URL per channel.** The same link pasted five times is
  one row, dated at its first appearance, or it becomes noise.
- **Only links in messages**, not in link previews of other content.
- **Deleted messages remove their link entries**, same as uploads — the
  kind-40099 tombstone path in `channelFiles.ts` already handles this.
- The composer's existing supersedes prompt should offer **link entries as
  candidates too**, so a new upload can supersede an old link and vice versa.

### Testing

Pure logic in a `.mjs` sibling with `.d.mts`, per the repo pattern. Cases:
dedupe by URL, naming fallback order, deleted-message removal, a link
superseding a file, a file superseding a link, malformed URLs ignored.

---

## Drive upload — **built**

### How it plugs in

One seam, not a parallel pipeline. `routedMediaUpload.ts` is signature-
compatible with `uploadMediaFile`, so `useMediaUpload.ts` and
`backgroundMediaUploadStore.ts` each swap a single import and keep every
behaviour they already had — cancellation, byte progress, slot ordering, the
deferred video queue. Nothing in the composer changed.

A Drive upload still returns an attachment descriptor, because that is how it
travels through composer state. It is marked `external`, which
`imetaMediaMarkdown.ts` reads to do two things:

- **emit no `imeta` tag** — there is no relay blob and no sha256 to assert, and
  fabricating one would be a lie the Files tab and the download path both act on;
- **always render as `[filename](url)`**, never inline media — `<video>` pointed
  at a Drive viewer page is a permanently broken player.

The result is that a Drive upload posts exactly what a person pasting a
labelled link would post. The Files tab then lists it as a link entry named
after the file, because the naming chain above reads that label. Version
chains, deletion and the supersedes picker all work with no further code,
because they were already event-based.

### Routing rule

At attach time in the composer:

| File | Route |
|---|---|
| Video or audio, any size | **Drive, always.** Never the relay. |
| Anything else over `DRIVE_UPLOAD_THRESHOLD_BYTES` (5 MB) | **Drive.** |
| Everything else | Relay upload, unchanged. |

Video and audio detection reuses `features/messages/lib/videoFileType.ts`,
which already maps extensions to mime types.

The threshold is a **named constant**, not a setting. One place to change, no
settings surface to build, tune it once real usage exists.

### No "do you want to use Drive?" prompt

The original ask was for the composer to *offer* Drive on a large file. It does
not, and the reason is that there is no second option to offer: direct sharing
of these files is not permitted, so a dialog whose only outcomes are "Drive" and
"cancel the upload" is a confirmation step wearing a choice's clothes. Routing
is silent, and the progress bar shows the same thing it always did.

### When Drive is not connected

**Block, and say why.** No silent fallback to relay upload — that would quietly
reintroduce exactly the failure this routes around, and for video it would fail
anyway.

The error names Settings → Voice, where the Google account connection already
lives: "Video, audio and files over 5 MB are shared through your Google Drive.
Connect your Google account under Settings → Voice to send this."

**Connected-for-Meet is not connected-for-Drive.** Anyone who connected before
this shipped holds a refresh token minted without `drive.file`; it refreshes
perfectly and then 403s on the first Drive call. `google_access_token` returns
the granted scope so this is caught up front and answered with "reconnect it",
rather than a raw API error halfway through a 200 MB upload.

### Where files land

**A `Buzz uploads` folder in each person's own Drive.** Created on first use.

This is deliberate: it works with the narrow `drive.file` scope, and each
person's uploads count against their own quota rather than a shared pool
nobody is watching. The cost is that files are scattered across individual
Drives — acceptable, since the Buzz message is the index, not the folder.

### Sharing — **no code**

The original design called one `permissions.create` per upload:

```
{ type: "domain", domain: "k2alpha.ai", role: "reader" }
```

**This was dropped, deliberately.** On 2026-08-21 the Workspace default was set
to "anyone in k2alpha.ai can access the item if they have the link", so a file
created by Buzz already carries the permission the call would have set. Writing
it anyway would be code that never changes an outcome — the worst kind, because
it looks load-bearing and would be preserved through future edits by people who
do not know it is inert.

If the default is ever narrowed, `google_meet/drive.rs` is where the call goes,
and its module comment says so.

**Domain sharing is confirmed available.** Verified 2026-08-21 in the Drive
share dialog: "k2alpha.ai — anyone in this group with the link" is offered as a
general-access option, so the `permissions.create` call above will succeed.

**The Workspace default was then set to exactly this** — "anyone in k2alpha.ai
can access the item if they have the link" — on 2026-08-21. Newly created files
already carry the permission we want, which makes the explicit call above a
safety net rather than the mechanism. Keep it: a default can be changed later,
and per-file permissions set at upload survive that change.

**One case still to handle.** Anyone signed in with a personal Gmail account
rather than a k2alpha.ai one has no domain to share to, and the call will fail
for them. Surface a clear error rather than posting a link nobody can open.

### Scope

**`drive.file` only.** The app can create files and manage what it created —
nothing else in the user's Drive. It is not a restricted scope, so it needs no
Google security review, and the consent screen is one people will accept.

Add it to the **existing** Google OAuth client used by Meet, not a new one.
Users re-consent once. The PKCE loopback flow, keyring token storage
(`SecretStore::shared`), build-time client ID/secret via `option_env!`, and
`release.yml` secret wiring all already exist in `google_meet.rs` — this is a
second set of commands against the same plumbing, not a new integration.

### Uploads are resumable

Drive's simple upload tops out around 5 MB — precisely our threshold. Every
file taking this path uses the **resumable** endpoint: open a session, PUT in
8 MiB chunks (Drive rejects any non-final chunk that is not a multiple of
256 KiB), emit progress after each one.

Two things about that endpoint that will look like bugs to the next reader, and
are commented at the code:

- Drive answers every intermediate chunk with **308 Resume Incomplete**, which
  `reqwest` reports as an ordinary non-success status. 308 is the success case
  for all but the last chunk.
- A zero-byte file still needs one request (`Content-Range: bytes */0`), or the
  session is never completed and Drive holds an empty reservation.

Progress is emitted on `media-upload-progress`, the **same event the relay
upload path uses**, so the composer's existing progress bar works unchanged.

Not yet built: actual resume-after-failure. A dropped connection restarts the
upload rather than continuing from the last acknowledged byte. The session URI
that would make that possible is already in hand, so it is a small addition
when it starts to matter.

---

## Why not folder browsing

Listing an existing Drive folder requires `drive.readonly` or `drive`. Both are
**restricted scopes** under Google's policy: a published app needs a security
assessment, and the consent screen carries a warning that makes people hesitate.

That is a large, ongoing cost for the least valuable of the four asks. Drive is
one click away in a browser, already organised the way its owner wants.

If it is ever wanted, the cheap version is a link to the folder in Drive, not a
file browser inside Buzz.

---

## Risks

- **Link rot.** A Drive link in a channel outlives the file. If someone deletes
  or unshares it, the Files tab shows an entry that leads nowhere. Uploads to
  the relay do not have this problem. Worth a "can't access this" state rather
  than a dead link.
- **Leaving the team.** Files in a departed colleague's Drive may vanish or
  become inaccessible when their account is suspended. A shared team folder
  would avoid this — the tradeoff we consciously took for quota simplicity.
- **Sharing is domain-wide, not channel-scoped.** Anyone at k2alpha.ai can open
  a file shared into a private channel if they have the link. Acceptable for an
  internal team; would not be for external collaborators.
- **Two file systems.** Some files live in the relay, some in Drive. The Files
  tab hides the seam, but download behaviour, permissions and deletion all
  differ underneath.

---

## Generalising this

Offered as-is, because it solves a real problem well for the teams it fits and
waiting for a perfect abstraction would have meant shipping nothing. For anyone
wanting to widen it, here is the work, in the order it matters:

1. **Set permissions explicitly instead of assuming the domain default.** This
   is the only change that fixes a *silent* failure rather than an inconvenience.
   Workspace accounts get `{type: "domain", domain: <the user's domain>, role:
   "reader"}`; personal accounts get `{type: "anyone", role: "reader"}` behind a
   clear warning, because that is a genuine disclosure; either failing surfaces
   an error and does not post the link. Roughly half a day, and it makes the
   feature survive an admin changing the default later.
2. **Give Drive its own home in Settings.** It currently borrows Meet's
   connection, so you cannot have one without the other. Either a shared "Google
   account" section, or its own.
3. **Make the threshold configurable**, or argue the constant properly. 5 MB was
   chosen because it is where Drive's simple upload endpoint stops working, which
   is a coincidence worth noticing but not a product justification.
4. **Cover the Drive path in e2e.** It is deliberately gated off today (see
   below), so the routing rules have unit tests and the upload itself has none.

The seam is already in the right place for a pluggable backend:
`routedMediaUpload.ts` is the single decision point, and the `external`
attachment flag in `imetaMediaMarkdown.ts` is the primitive any
storage-elsewhere implementation needs — no `imeta` tag, always a link, never
inline media. S3, Dropbox or a self-hosted bucket would slot in beside Drive
without touching the composer.

---

## Two things the build changed that are worth knowing

**`lib.rs` had to be extracted before this could land — and then un-extracted.**
Registering two Tauri commands needs two lines, and `lib.rs` was past the
repository's 1000-line ceiling, so the 320-line `generate_handler!` list moved
into a `command_registry.rs` macro.

That lasted one day. Upstream's 0.5.18 shipped `lib.rs` at 932 lines *and* 13
new command registrations, which would have landed inside a 338-line conflict —
and a lost registration fails only at runtime, invisible to `tsc`, `cargo check`
and the test suite. The extraction was reverted during that catch-up and the
file deleted. The lesson is not "don't extract"; it is **don't extract the one
block upstream edits every release**.

**Drive routing is off under e2e unless a spec opts in.** Several existing
specs upload a 16 MB file or a video specifically to assert the *relay* path,
and the suite has no Google account. The flag mirrors the existing
`deferredComposerUploads` escape hatch. The consequence is real and should not
be forgotten: the Drive path has unit coverage over its routing rules and no
end-to-end coverage at all.
