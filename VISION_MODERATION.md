# 🛡️ Buzz Moderation — Your community, your rules

> Someone spams #general at midnight. A member taps **Report** — a category, an optional note, done. The report doesn't appear in anyone's feed; it lands in a queue only the community's owners and admins can see. In the morning an admin opens the queue, finds three reports against the same account, deletes the messages, and times the account out for a day. The room sees an honest marker where the spam was. The author gets a message explaining why. The reporter gets a message saying it was handled. Nobody else saw anything.

A Buzz community is a trust group with its own rules, and rules only matter if the people who own the room can enforce them. Buzz moderation gives community owners and admins the full loop: members report, the community's own owners and admins see and act, the relay enforces, and everyone affected hears the truth about what happened. The relay provides the mechanics — queue, authority, enforcement, audit, notices. The community provides the judgment. Buzz doesn't decide what your rules are; it makes sure you can actually have them.

Most of the nostr ecosystem treats moderation as **admission policy** — allow lists, block lists, a hook that rejects an event at the door. Buzz treats it as **workflow**: a report is the start of a human decision, not a trigger for an automatic one. That difference is the whole design.

---

## Two Layers, Not One

Moderation splits the way it does on every serious platform:

**Community moderation** — subjective, per-community rule enforcement. Your owners and admins decide what's spam in *your* community, what crosses *your* line, who gets a second chance. This layer belongs to the community and never reaches past it: an admin's authority ends at the community boundary, structurally, because every moderation decision is scoped to the tenant it was made in.

**Platform safety** — the severe class: illegal content, network-level abuse, legal reporting obligations. That is never delegated to community admins. A community owner or admin can **escalate** a report upward, and the escalation is recorded durably for the platform operator's safety process. The community layer is the front line; the platform layer is the backstop.

This document is about the first layer. The second has its own lane.

---

## What You See

**As a member**, every message has a Report action. Pick why — spam, profanity, illegal content, impersonation, or another supported reason — add context if you want, send. Your report is private: it is never broadcast, never stored as a public event, never visible to the person you reported. It goes to the people who can act on it, and only them.

**As an owner or admin**, you have a queue. Reports arrive grouped by target, newest first, with the reporter's identity visible to you — accountability runs both ways — and never to the reported author. From the queue you act in one motion: dismiss, delete the message, kick, timeout, ban, or escalate. Reasons travel where they should — to the audit trail, to the tombstone, to the restricted user — without exposing private report context to the room.

**As the room**, a removed message leaves an honest tombstone — "removed by a community moderator," with a sanitized reason — instead of a silent hole. The room learns that the rules are real without republishing the offense.

**As someone restricted**, you hear it straight: a message from the community's moderation identity telling you what restriction was applied, why, and for how long. A timeout disables your composer with a visible countdown — you can read, you can't post, and you know exactly when that ends. No silent write-drops, no shadow bans, no guessing.

**As the reporter**, you hear the outcome. The loop closes. Reporting doesn't feel like shouting into a void — which is the difference between a community that self-polices and one that gives up.

---

## The Mechanics That Matter

- **Reports are signals, never triggers.** No user report auto-removes anything. Reports are gameable; human judgment is the gate. The queue aggregates and an owner or admin decides.

- **Reports are private structural state.** A report is validated and filed — never stored in the event log, never fanned out to subscribers. Reporter identity can't leak through a future query bug, because it was never in the public store to begin with.

- **Moderation actions are signed commands.** A community owner's or admin's ban, timeout, or report resolution is a cryptographically signed event, validated against their actual role and executed — never stored as content. Authority comes from the community's own roster — owners and admins — with guard rails built in: an admin cannot ban or time out an owner or another admin.

- **Enforcement lives at the identity seam.** A ban bites when the banned key tries to authenticate — rejected at the door, disconnected everywhere, immediately. A timeout is a write-block with a stated expiry. Enforcement isn't scattered through the codebase as filters; it happens where identity is established, which is why it can't be sidestepped.

- **The important decisions are audited.** Bans, timeouts, report dismissals, escalations, and report resolutions write durable audit rows — who, what, whom, why, when — with the decision recorded separately from its enforcement, so the trail never claims something happened that didn't. Message removals also leave visible tombstones for the room. The full report record (including reporter identity and notes) stays moderator-only; the public sees only the sanitized reason.

- **The wire uses nostr where nostr has the right primitive.** Reports are NIP-56. Group roles and membership actions are NIP-29. Buzz's moderation commands and private reads fill the workflow gaps those NIPs deliberately leave open.

---

## Honest Edges

**Escalation is a hook today, not a pipeline.** Escalating writes a durable, queryable record for the platform operator — but the platform-side inbox that consumes it is a separate build. The substrate is there; the tooling above it comes next.

**Two roles, not three.** Owners and admins moderate. There is no volunteer-moderator tier yet — deliberately. Authority is structured as capabilities, so adding a moderator tier later is a policy change, not a rewrite. We'd rather ship a loop that works and grow the org chart when communities ask for it.

**Notices are best-effort.** The DMs that close the loop never block enforcement — a ban lands even if the notice fails. Enforcement is the promise; notification is the courtesy. A later platform-escalation pass should also make escalated reports say exactly that, instead of reusing the generic handled message.

**No automod.** Nothing scans content before it posts. Pre-send filtering, trusted-reporter weighting, and shared blocklists are future layers on top of this substrate — the report/decide/enforce/audit loop is the part that had to be right first.

---

## The Point

A community you can't moderate isn't yours — it just has your name on it. The relay is the workspace; moderation is what makes it *governable* by the people it belongs to. Judgment stays human. Enforcement is structural. Everyone affected hears the truth.

---

*Buzz 🐝 — your community, your rules.*
