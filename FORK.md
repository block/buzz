# Buzz k2alpha — what this fork is

This repository is a fork of [`block/buzz`](https://github.com/block/buzz),
maintained by the k2alpha team and used daily by ~40 people on Windows.

Everything upstream does, this does. This document covers only what is
**different**, so you can decide whether to run this build, take a patch from
it, or contribute.

Current release: **`0.5.14-6`**, based on upstream `desktop-v0.5.14`.
Windows installers are published under
[Releases](https://github.com/ranjank2alpha/buzz/releases).

---

## Why the fork exists

Upstream Buzz is a general platform. We run it as the primary communication
tool for a working team, which surfaced a different set of problems: files
getting confused between versions, an activity list nobody trusted, and a
release pipeline that had to produce signed Windows builds without Block's
infrastructure. Those are the things we built.

We are not trying to diverge. Several changes here are candidates to go back
upstream — see [`docs/upstream-candidates.md`](docs/upstream-candidates.md).

---

## What this fork adds

### File versioning

The largest addition. A file can be marked as a newer version of an earlier
one, and every surface then tells you which is current.

- At upload time the composer asks whether the file supersedes an existing one,
  with fuzzy filename matching that recognises `report-v2.pdf`, `report (1).pdf`,
  `deck FINAL.pptx`, `budget_2026_rev2.xlsx`, bounded to files from roughly the
  last two months.
- The current version shows "supersedes N earlier versions"; older ones show
  "Outdated — view latest" and jump to the head of the chain, not one step back.
- The Files tab collapses a version set into a single row.
- Version chains are **derived at read time**, never stored. Nostr events are
  immutable, so a message cannot know it has been superseded — that fact lives
  in a later message. Chain resolution handles cycles, forks, self-supersedes
  and missing parents.

Pure logic with tests: `fileVersionChains.mjs`, `supersedesRanking.mjs`.

### Inbox as three lanes

Rebuilt so the Inbox shows only what is addressed to you: **mentions**,
**threads you are part of**, and **workflow approvals**. Ordinary channel
traffic belongs to the sidebar. Mentions supersede threads, so a reply that
@-mentions you is counted once.

This design was reached by building the opposite first — a row per channel with
unread counts — shipping it, using it, and reverting it. The history is
recorded in `CONTEXT.md` so nobody rebuilds it by accident.

### Sidebar

- Collapsed sections roll up what is inside them: a count when someone has
  mentioned you or sent a DM, a dot for ordinary activity, nothing when quiet.
  Without this, folding channels away hides them completely.
- Unread dot visibility fixed on the active row, where it was drawing
  primary-on-primary and effectively invisible.

### Global mentions

`@channel` notifies everyone in a channel; `@here` only those currently online.
`@channel` reaches people even in a channel they have muted — it exists for the
things you would want pulled out of a mute — and `@here` never does.

An `@here` you missed while offline does not accumulate as urgent when you come
back, which is the whole distinction from `@channel`.

Both are a single marker tag on the message rather than a `p` tag per member,
so the audience resolves against *current* membership: someone who joins an
hour later still sees the announcement as addressed to them.

Two things to know. The words are typed, not offered by autocomplete — the
mention hook carries debouncing, personas and teams, and threading synthetic
entries through it was a larger change than the feature needed to start
working. And there is currently **no permission gate**: anyone can `@channel` a
forty-person room. The size-based gating logic exists and is tested
(`canUseMentionScope`, admins-only above 32 members, following WhatsApp's
design) but is deliberately unwired pending real usage.

### Google Meet

Start an instant Meet from a channel or DM and post the join link. Each user
connects their own Google account (OAuth 2.0 + PKCE, loopback redirect). Added
because Buzz's built-in Huddle uses custom relay-fanout audio rather than WebRTC,
and adding real video was blocked by the relay's per-kind event allowlist. Meet
sidesteps that entirely — the join link is an ordinary message.

### Release pipeline

A single-job Windows build producing a signed NSIS installer plus a Tauri
updater manifest, published to this repository's Releases on a `v*` tag.
Deliberately not upstream's multi-platform workflow, which hardcodes publishing
to `block/buzz`.

### Smaller things

- Windows toast notifications via AUMID/Start-Menu-shortcut repair.
- Full release history in Settings → Updates, including upstream Buzz releases
  interleaved by date up to the version this build is based on.
- "What's new" splash keyed on the whole version string.
- Google SSO with deterministic key derivation (on the `google-sso` branch).

---

## Running it

Prebuilt Windows installers are on the
[Releases page](https://github.com/ranjank2alpha/buzz/releases). The app
auto-updates from there.

Building from source is identical to upstream — see the main
[README](README.md) and [ARCHITECTURE.md](ARCHITECTURE.md). Two things specific
to this fork:

- **Two Cargo workspaces.** The root `Cargo.toml` excludes `desktop/src-tauri`,
  which has its own. `cargo check --workspace` must be run from *both*
  directories; running it in one silently skips the other's crates.
- **Build-time secrets.** Google Meet and the updater read credentials from
  environment variables at compile time (`option_env!`, so unconfigured builds
  still compile and report the feature as unavailable). See
  [`docs/`](docs/) for the specific variables.

---

## Known limitations

Stated plainly, because they are the things most likely to surprise you:

- **No voice notes**, no video messages, no mobile client in practice.
- **No autocomplete for `@channel` / `@here`**, and no permission gate on them.
- **The Files tab lists top-level channel messages only.** A file attached
  solely inside a thread reply will not appear there — the relay excludes
  thread replies via a `thread_metadata` join.
- **Deleting a middle version breaks a chain.** The link between the remaining
  ends only ever existed in the deleted message's tag.
- **The relay soft-deletes.** Deleted content keeps returning from queries;
  clients honour tombstone events. "Really gone" is not achievable today.
- **`identifier` in `tauri.conf.json` is still upstream's** (`xyz.block.buzz.app`).
  On Windows this derives the upgrade code, so it collides with an official
  Buzz install on the same machine. Fixing it is a one-time breaking change we
  have deferred.
- **Auto-update has now been observed working**, but the pipeline is young.

---

## Contributing

See [`docs/roadmap-and-research.md`](docs/roadmap-and-research.md) for what we
think should be built next and why, including community research into what
users of Slack, WhatsApp, Discord and Teams actually complain about. Items
there are unclaimed and reasoned-through rather than a wishlist.

If you want the shortest path to something useful: the open items are listed
with their constraints, and every non-trivial piece of logic in this codebase
has a `.mjs` sibling with tests, so the pattern to follow is visible.

Upstream's [CONTRIBUTING.md](CONTRIBUTING.md) and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) apply here too.
