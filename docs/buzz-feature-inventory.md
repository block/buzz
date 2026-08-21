# Buzz — feature inventory

A complete map of what this product does today, for competitive gap analysis.

**What this is.** Buzz is a desktop team-chat application (Tauri 2 + React,
Windows/macOS/Linux) built on the Nostr protocol. Messages are signed events on
a relay rather than rows in a vendor database. This repository is `k2alpha`, a
fork of the open-source `block/buzz` project, currently at version `0.5.17-1`
(based on upstream `desktop-v0.5.17`, plus fork-specific work).

**Who uses it.** A team of ~40 senior professionals, predominantly on Windows,
whose prior habit was WhatsApp.

**Why this document exists.** To be fed to an analysis tool alongside community
discussion (Reddit and similar) about Slack, WhatsApp, WeChat, Discord and
Google Chat, to identify what users of those products complain about that Buzz
does not yet address. Everything below is *implemented and shipping* unless
explicitly marked otherwise.

---

## 1. Messaging core

- **Channels** — public and private. Membership-based. Channel types, per-channel
  permissions, and templates for creating pre-configured channels.
- **Direct messages** — 1:1 and group.
- **Threads** — replies grouped under a parent message rather than filling the
  main timeline. Threads can be explicitly followed or muted, independent of
  the channel. A focused thread drawer and a side thread panel both exist.
- **Global mentions** — `@channel` notifies every member; `@here` only those
  currently online. `@channel` pierces a muted channel, `@here` does not, and an
  `@here` missed while offline does not persist as urgent. Carried as one marker
  tag resolved against current membership, not a `p` tag per member. Typed
  rather than autocompleted, and currently ungated — anyone may use either.
- **Message composition** — markdown, formatting toolbar, emoji picker with
  autocomplete, @-mention autocomplete, #channel autocomplete, inline image
  editor before sending, drag-and-drop and paste attachments, upload progress.
- **Editing and deletion** — messages can be edited after sending and deleted by
  their author. Deletion is a tombstone event; the relay soft-deletes.
- **Reactions** — emoji reactions, including custom emoji.
- **Pinned messages** — up to 3 per channel or DM, shown in a bar above the
  timeline.
- **Forwarding** — multi-select messages (Ctrl/Cmd+click) and forward to another
  person or channel.
- **Permalinks** — copyable links to individual messages.
- **Drafts** — persisted per conversation, with a dedicated Drafts panel and a
  send-from-drafts flow.
- **Ephemeral channels** — channels with a TTL that expire.
- **Day dividers and unread markers** in the timeline.

## 2. Inbox (activity centre)

- Three lanes, mutually exclusive: **mentions**, **threads you are part of**,
  and **workflow approvals waiting on you**.
- Thread membership rule: threads you started, replied in, were mentioned in, or
  explicitly followed. Muted threads excluded.
- A mention inside a thread counts once, as a mention.
- Defaults to unread-only; a toggle reveals read history.
- Two-pane list/detail with inline reply and per-item drafts.
- Read state is shared with the channel view — reading in either place clears it
  in both.
- Deliberately does **not** list ordinary channel/DM activity; that is the
  sidebar's job. (This was tried in one release and reverted after use.)

## 3. Files

- Any file type as an attachment. Images and video render inline.
- **In-app preview** for PDF, Word (.docx), Excel (.xlsx), PowerPoint (.pptx),
  Markdown and plain-text/code. PowerPoint fidelity improves when LibreOffice is
  installed.
- **Files tab** per channel — every file shared, with uploader, date, time, size.
- **Links as file entries** — any web link pasted into a channel message becomes
  a row in the Files tab, deduplicated to one entry per URL and dated at its
  first appearance. Named by Google surface (`Google Doc`, `Google Drive file`)
  or the last readable path segment; never a bare opaque id. A sender-supplied
  markdown label outranks all of it, which is how Drive uploads appear under
  their real filename. Opens in the browser. A link and a file can supersede
  each other, because the version tag references an event rather than a file.
- **File versioning** — at upload time, Buzz asks whether the file is a new
  version of an existing one, with fuzzy filename suggestions (recognises
  `report-v2.pdf`, `report (1).pdf`, `deck FINAL.pptx`, `budget_2026_rev2.xlsx`)
  limited to files uploaded in the last ~2 months. Current versions show
  "supersedes N earlier versions"; older ones show "Outdated — view latest" and
  jump straight to the newest. Version sets collapse into one row in the Files
  tab. Correcting a mistake means deleting the message that carries the link.
- **Google Drive routing** — files over 5 MB, and all video and audio, upload to
  a "Buzz uploads" folder in the sender's own Drive (`drive.file` scope, reusing
  the Meet OAuth connection) and post as a labelled link rather than a relay
  attachment. Resumable chunked upload with byte progress. Blocked with an
  explanation when Drive is not connected — no relay fallback. No per-file
  sharing call; the Workspace domain default covers it.
- **Diff viewer** for code/text changes.
- Known limitation: the Files tab lists top-level channel messages only; a file
  attached solely inside a thread reply does not appear there.

## 4. Calls and real-time

- **Huddle** — built-in voice channel (custom relay-fanout audio, not WebRTC;
  quality is known to be weak). Huddle transcripts exist.
- **Google Meet** — start an instant Meet from a channel or DM and post the join
  link as a message. Each user connects their own Google account via OAuth.
- **Presence** — online/offline indicators.
- **User status** — custom status with emoji.
- **Typing indicators**.

## 5. Agents and automation

- **Agents** — assistants added to channels, running as local processes on the
  owner's machine. Configurable harness, model provider, credentials, and a
  permission model for who they respond to (`owner-only`, `allowlist`,
  `anyone`).
- **Agent personas and teams** — named bot identities, instance name pools,
  parallelism settings.
- **Agent memory** — persistent memory per agent.
- **Agent session threads** — an agent's working session rendered as a thread,
  with activity cards.
- **Agent snapshots**.
- **Workflows** — approval requests routed to specific people, with approval
  cards, a detail panel, and an Inbox lane.
- **Mesh compute** — distributed compute settings.

## 6. Organisation and knowledge

- **Projects** — repositories, issues, pull requests, and git status events
  rendered natively in the timeline (PR opened/merged/closed/draft).
- **Forums** — long-form threaded discussion channels, distinct from chat.
- **Canvas** — a collaborative document surface attached to a channel.
- **Pulse** — an activity/notes view.
- **Search** — full-text message search (Typesense-backed).
- **Reminders** — set a reminder on a message, with a dedicated panel.
- **Link previews** — with per-message suppression.
- **Video review comments** — timestamped comments on video attachments.

## 7. Notifications

- Native desktop notifications on Windows, macOS and Linux.
- Per-channel and per-thread muting. A muted channel still shows an unread
  indicator but produces no notification.
- Unread indicators in the sidebar, split between top-level channel activity and
  thread activity, with per-channel counts and a high-priority tier (DMs,
  mentions, broadcasts).
- Mark channel read/unread, mark all read, mark individual messages read/unread.
- Read state is a shared cross-device marker system (NIP-RS), so reading on one
  device advances it everywhere.

## 8. Identity, community and moderation

- **Nostr keypair identity**, stored in the OS keyring.
- **Google SSO** with deterministic key derivation (fork-specific).
- **Communities** — multiple communities with a switcher, icons, membership,
  and hosted community support.
- **Community members** directory.
- **Moderation** — report a message, time out a user with a duration, moderation
  actions from channel management, composer timeout banner.
- **Identity archive** and **local archive** — local retention of content.
- **Mobile pairing** — pair a Buzz mobile client from settings.

## 9. Platform and shell

- **Buzz Term** — a terminal inside the app.
- **Themes** — many light and dark themes, accent colour picker.
- **Custom emoji** — upload and use in messages and reactions.
- **Auto-updater** — checks periodically, downloads and installs in place; full
  release history in Settings → Updates, including upstream Buzz releases
  interleaved by date up to the version this build is based on.
- **What's New splash** on first launch after an update.
- Onboarding flow.

---

## What Buzz does NOT have

Stated explicitly, since gap analysis depends on it:

- **No voice notes / audio messages.** No recording, no inline playback, no
  playback speed control.
- **No `@everyone`, no user groups / roles for mentions** (`@design-team`).
- **No permission gate on `@channel` / `@here`**, and no autocomplete for them.
- **No video messages.**
- **No mobile app in practice.** Mobile pairing exists; the product is used on
  desktop.
- **No scheduled send.**
- **No message recall window** (deletion is permanent, not time-boxed).
- **No read receipts visible to the sender** — read state is tracked per user
  but never shown to others.
- **No delivery/seen indicators** (no ticks).
- **No disappearing messages.**
- **No polls or surveys.**
- **No saved-messages / favourites collection** separate from reminders.
- **No status/broadcast channel** in the WhatsApp sense.
- **No calendar integration.**
- **No task/deadline tracking** beyond project issues.
- **No screen sharing** (Google Meet handles this externally).
- **No end-to-end encryption** of channel messages.
- **No guest or cross-organisation access** (no Slack Connect equivalent).
- **No translation.**
- **No stickers or GIF search.**
- **No message formatting beyond markdown** (no rich embeds, no code execution).
- **No analytics or usage dashboards.**

---

## Constraints worth knowing for feasibility

Any suggested feature should be weighed against these:

1. **Events are immutable and signed client-side.** There is no server that can
   modify or hold a message. Anything requiring server-side scheduling,
   delayed delivery, or content rewriting needs new relay infrastructure.
2. **The relay soft-deletes.** Deleted content keeps returning from queries;
   clients honour tombstone events. "Really gone" is not currently achievable.
3. **New event kinds are constrained.** The deployment's relay uses a
   per-kind allowlist, so features requiring a new event kind need relay-side
   changes — this is why WebRTC signalling for in-app video was abandoned.
4. **Agents run locally on the owner's machine**, with that owner's file and
   shell access. Anything agent-driven inherits that trust boundary.
5. **Small team, no dedicated infrastructure staff.** Features requiring new
   hosted services carry disproportionate ongoing cost.

---

## Questions worth answering from community discussion

1. What do teams who left Slack, Teams or Discord say they left *for*, and what
   did they miss afterwards?
2. Where teams tried and failed to move off WhatsApp, what specifically caused
   the reversal?
3. Which features do people repeatedly request that vendors have refused to
   build, and why?
4. What do senior/professional users (as opposed to consumer or developer users)
   complain about most in team chat?
5. Which of the "does not have" items above appear in complaints, and which are
   never mentioned?
