# Bestie list protocol (`bestie-list`)

Bestie reminders and to-dos live in the Bestie DM RHS (localStorage, scoped by
relay / owner / agent). Mutations happen three ways:

1. **RHS UI** — category slide `+` (reminders include a due datetime picker and
   One-off / Daily repeat).
2. **Natural language** — user messages like `Remind me to … in 20 minutes` or
   `Add a todo: …` are applied client-side (and a short turn hint is appended so
   the agent can acknowledge with a fence). **Exception:** a bare clock without
   AM/PM (e.g. `at 8:36`) does **not** auto-create — the client stores a pending
   confirm; the agent asks AM vs PM **in the same thread** (not a new top-level
   DM); when the user replies `PM` / `AM` (or `8:45 PM`), the **client** resolves
   `dueAt` and creates the reminder. Agent fences are reconciled when prose says
   PM/AM but `dueAt` disagrees (e.g. next-morning AM).
3. **Agent fence** — Bestie agent replies may include:

```bestie-list
{"op":"add","items":[{"kind":"reminder","text":"Run the nightly report","dueAt":1735689600000}]}
```

`dueAt` is unix **milliseconds** (Date.now()-style; seconds-scale values are coerced on apply; ISO-8601 strings also accepted). Optional
`repeat: {"kind":"daily"}` or `{"kind":"weekly","weekday":1}` (0=Sun). Ops:
`add`, `complete`, `remove`. Completing a recurring reminder advances `dueAt`
instead of marking done.

**Crystallized reminder text:** store a clean imperative — capitalize the first
letter; drop a leftover leading `to`/`for`. Example: user said
`remind me to run the nightly report` → stored / fenced
`Run the nightly report` (time only in `dueAt`, never in text).

Due reminders are detected by the ~5 minute wake loop (plus a one-shot timer for
the next dueAt). A due nudge shows the footer `!` badge, auto-opens the Bestie
popover, and renders the amber Reminder banner (title **Reminder**, body = exact
reminder text). Banner actions: Dismiss, +15 min, +1 hour; square-arrow opens
the Reminders RHS/sheet.

Client dedupe: open items with the same kind and **core text** (time phrases
stripped) are not added twice when due times match within 120s, or when the
first item was created in the last 10 minutes; close dueAt + similar wording
also matches. Prefers a **near** client-NL `dueAt` over a far fence epoch, and keeping/upgrading the row that has `dueAt` (covers NL +
agent fence on one ask even when wording differs).

## Live list state (each user turn)

On every Assistant DM / popover user send, desktop appends a `[Bestie live lists]`
turn hint with the current **owner-scoped** RHS snapshot: open to-dos, open
reminders, and an enabled-jobs summary (from localStorage). The agent must answer
questions like “what's on my todo list?” from that block — not session memory —
and must not claim it cannot query live state. The hint is stripped for UI
display; NL intent parsers also strip outbound hints before matching.

