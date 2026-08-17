# Roadmap and research

What we think should be built next in this fork, and the evidence behind it.
Nothing here is claimed. If you want to pick something up, open an issue saying
so and we will keep out of your way.

This is deliberately reasoned rather than a wishlist: each item says what the
evidence is, what it costs, and where it collides with the architecture.

---

## What the research actually said

We surveyed community discussion and vendor documentation about Slack,
WhatsApp, Discord, Teams and Matrix in August 2026, looking for what users
complain about rather than what vendors advertise. Four findings shaped the
list below.

**1. Notification fatigue is the single most common complaint about Slack.**
Ahead of everything else. Any feature that generates notifications has to ship
with its controls, not after.

**2. The second complaint is knowledge loss** — not being able to find old
messages, and chat being a poor place to keep anything long-term. Threads are
where decisions go to be forgotten.

**3. Teams that tried to leave WhatsApp sometimes reverted.** One documented
case: people ignored Slack and had to be reminded *on WhatsApp* to check it.
The failure was not a missing feature, it was a missing habit — worth
remembering before assuming any feature drives adoption.

**4. Voice is the largest single capability gap.** WhatsApp carries billions of
voice messages a day; Slack was at roughly 1.46 million *minutes* a week, and
declined to build native voice notes back in 2018, still restricting what apps
can do with audio. Third-party integrations take 7–8 clicks to record and offer
no playback speed control. That gap has stayed open for years.

Sources are listed at the end.

---

## Open items

### Voice notes — highest value, unclaimed

Record, send and play back short audio inline. **Not** transcription — that is
a separate, much more expensive feature and coupling them has killed this idea
before.

*Why:* the largest capability gap, and the one with the clearest evidence of
demand. Especially relevant for teams whose habit is WhatsApp.

*Cost:* moderate. Recording and Opus encoding in Tauri, then the existing
attachment path carries it — it is a file with a mime type. No new event kind,
no relay change. The work is the player UI: waveform, scrub, speed control.

*Watch out for:* the temptation to bundle local Whisper transcription. That
adds a sizeable model to a binary already shipping six sidecars, plus
per-platform inference. Ship voice first.

### Thread-to-spec — highest strategic value, unclaimed

A context-menu action on a thread that has an agent extract the decisions,
trade-offs and open questions into the channel's Canvas or a project issue.

*Why:* directly attacks finding 2. All three pieces already exist — thread
history is retrievable, agents run locally with an inference endpoint, and both
Canvas and issues have write paths. This is composition, not new
infrastructure.

*Cost:* moderate, but the value depends entirely on output quality, which is
unknowable until tried. Prototype crudely first — a command that posts a
summary back into the thread as an ordinary message, no Canvas write — and see
whether the output is worth wiring properly.

*Watch out for:* an LLM will sometimes state a rejected option as the decision.
Writing that into a Canvas as fact is worse than no summary, so a
preview-and-edit step is probably mandatory, and that is most of the UI work.
Agents are per-user and run on the owner's machine, so whoever triggers it pays
for it.

### Universal thread file indexing — unclaimed

Files attached inside thread replies do not appear in the channel's Files tab.

*Why:* architecture diagrams and log dumps shared deep in a thread become
unfindable.

*Cost:* higher than it looks. This is **not** a client-side indexing fix — the
relay excludes thread replies via a `thread_metadata` join
(`channelFiles.ts:149`). Closing it needs either a relay-side query change or a
separate per-thread sweep, which means one extra query per thread on a busy
channel.

### Message recall window — unclaimed

WhatsApp-style "delete for everyone" within a bounded time.

*Why:* the social contract matters more than the mechanism. A short recall
window is what makes people willing to type quickly.

*Cost:* deletion already exists. The interesting part is that the relay
soft-deletes, so this is cooperative rather than enforced — worth being honest
with users about.

### Scheduled send — **do not build as commonly specified**

Listed here to save someone the trip. The obvious implementation holds the
message locally and publishes at the target time. That only works if the app is
running — and the entire use case ("send at their 9am", "respect deep work
hours") means the sender is asleep with a shut laptop. It fails exactly when it
is needed.

A correct version needs relay-side hold-and-release, which is real
infrastructure. An honest smaller version is "remind me to send this", which is
a different feature and should be named differently.

### Deferred deliberately

- **Read receipts visible to the sender.** No evidence anyone is asking for it,
  and it carries a privacy decision that should be made on purpose rather than
  drifted into.
- **User groups / roles for mentions** (`@design-team`). Every platform has
  them and they are useful, but they need group creation, membership management
  and a sync story.
- **Mobile.** The honest largest gap, and far beyond a feature.

---

## Constraints any proposal must clear

This is a Nostr application, not a conventional client-server one. Four things
routinely invalidate otherwise reasonable ideas:

1. **Events are immutable and signed client-side.** No server can modify, hold
   or rewrite a message. Anything needing server-side scheduling or content
   rewriting requires new relay infrastructure.
2. **The relay soft-deletes.** Deleted content keeps returning from queries;
   clients honour tombstones. Deletion is cooperative.
3. **New event kinds are gated.** The deployment's relay uses a per-kind
   allowlist. This is why in-app WebRTC video was abandoned and Google Meet
   used instead — a feature needing a new kind needs relay-side changes.
4. **Agents run locally**, with the owner's shell and file access. Anything
   agent-driven inherits that trust boundary.

Derived state is the recurring pattern here. Because events cannot change,
things like "is this file outdated" are recomputed at read time from the whole
channel history rather than stored. Expect to write graph-walking code that
tolerates cycles and missing nodes, and expect it to be tested.

---

## How work is done here

Non-trivial logic lives in `.mjs` files with hand-written `.d.mts` type
siblings, so `node:test` exercises the exact source the UI runs without a
TypeScript loader. See `fileVersionChains.mjs`, `supersedesRanking.mjs`,
`channelUnreadRows.mjs` for the pattern.

Before shipping: `tsc --noEmit`, `biome check src`, the test suite, and
`check:file-sizes` (a 1000-line-per-file ratchet). When a file hits the
ratchet, extract something — do not squash lines behind format-ignore
directives. Two files are already at the wall for exactly that reason.

---

## Sources

- [Yac — Picking up the slack on voice messaging](https://www.yac.com/blog/picking-up-the-slack-on-voice-messaging)
- [Axios — Why we love voice notes](https://www.axios.com/2023/04/23/voice-memo-whatsapp-imessage-hinge-slack)
- [TechCrunch — 7 billion voice messages a day](https://techcrunch.com/2022/03/30/people-are-sending-7-billion-voice-messages-on-whatsapp-every-day)
- [DEV — Why we're moving from Slack and Teams to WhatsApp](https://dev.to/doozieakshay/why-were-moving-from-slack-and-teams-to-whatsapp-for-internal-communication-5885)
- [Workvivo — Slack pros and cons 2026](https://www.workvivo.com/internal-communications/slack-pros-cons/)
- [Pumble — Slack review 2026](https://pumble.com/reviews/slack-review)
- Mention-system survey with per-platform sources:
  [`global-mentions-research.md`](global-mentions-research.md)
