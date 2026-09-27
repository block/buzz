# Bestie list protocol (`bestie-list`)

Bestie reminders and to-dos live in the Bestie DM RHS (localStorage, scoped by
relay / owner / agent). Mutations happen three ways:

1. **RHS UI** — category slide `+` (reminders include a due datetime picker).
2. **Natural language** — user messages like `Remind me to … in 20 minutes` or
   `Add a todo: …` are applied client-side (and a short turn hint is appended so
   the agent can acknowledge with a fence).
3. **Agent fence** — Bestie agent replies may include:

```bestie-list
{"op":"add","items":[{"kind":"reminder","text":"Call mom","dueAt":1735689600}]}
```

`dueAt` is unix seconds (ISO-8601 strings also accepted). Ops: `add`,
`complete`, `remove`.

Due reminders are detected by the ~5 minute wake loop (plus a one-shot timer for
the next dueAt). A due nudge shows the footer `!` badge, auto-opens the Bestie
popover, and renders the amber check-in banner (not a normal DM).
