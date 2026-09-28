# Bestie Jobs protocol (`bestie-job`)

Jobs are scheduled prompts. When due, desktop wakes the Assistant agent and
sends the prompt as a **top-level Assistant DM user turn** (not a nudge banner,
not a second channel).

UI may say **Assistant**; protocol ids stay `bestie-job` / Bestie Jobs.

## Busy policy (Queue)

Job fires and due-reminder notifies share the Assistant DM session with
interactive chat. Managed agents set `BUZZ_ACP_MULTIPLE_EVENT_HANDLING=queue`
and `BUZZ_ACP_DEDUP=queue`, so when the agent is mid-turn (thinking) a due
notify **queues** and runs as its own top-level turn after the current turn
finishes — it must not Drop or Steer/interrupt the in-flight turn.

Do **not** create a parallel `#bestie-jobs` (or any second) channel for this.

Coffee + thread-summarize also stay on the Assistant DM as top-level turns.

## Schedule kinds

- `once` — `{ "kind": "once", "dueAt": <unix seconds> }`
- `interval` — `{ "kind": "interval", "everySeconds": 3600 }`
- `daily` — `{ "kind": "daily", "hour": 9, "minute": 0 }`
- `weekly` — `{ "kind": "weekly", "weekday": 1, "hour": 9, "minute": 0 }` (0=Sun)

## Chat confirmation protocol (required)

Unlike reminders, **chat must not instantly create** a job from casual NL or an
unconfirmed fence.

1. User asks to schedule a job → Assistant asks clarifying questions:
   schedule type `one-shot | interval | daily | weekly` and needed fields
   (when, interval length, time of day, weekday, prompt / what to run).
2. Before creating, Assistant presents an **exact approval plan**:
   - What will run (full prompt; message/mention someone if applicable)
   - When (concrete schedule)
   - Concrete targets when relevant (channel id, thread id, recipients, etc.)
3. Only after the user explicitly acknowledges/approves that plan → create.
4. Create via fence with `"confirmed": true` (see below).

RHS + manual add may still create directly when the form is complete.

## Agent fence

Create (only after user approval):

```bestie-job
{"op":"add","confirmed":true,"job":{"title":"Morning brief","prompt":"Summarize calendar","schedule":{"kind":"daily","hour":9,"minute":0}}}
```

Optional draft while clarifying (**does not create**):

```bestie-job
{"op":"draft","job":{"title":"Morning brief","prompt":"Summarize calendar","schedule":{"kind":"daily","hour":9,"minute":0}}}
```

Ops: `add` (requires `confirmed: true`), `draft` (no-op for storage), `update`, `remove`.
Unconfirmed `add` is treated as draft and does not create.

## Client NL

- Casual `Schedule a job…` / `Every N minutes run a job…` → **do not** auto-apply create.
  Attach turn hint so Assistant runs the confirmation protocol.
- Explicit cancel/disable/remove job by title → still applied from NL.
- Job approve phrases (e.g. `approve the job`, `yes create it`) → turn hint only;
  create still comes from the confirmed fence after the plan was shown.

## Deduping

Same title+prompt within 10 minutes is not added twice (confirmed fence only).
Each due slot (`jobId@dueAt`) fires once; recurring jobs reschedule `nextDueAt`.

## Live list state

Each Assistant DM / popover user turn also carries a `[Bestie live lists]` snapshot
(open to-dos, reminders, jobs summary). Job Q&A about “what jobs do I have?” should
use that block; see `BESTIE_LIST.md`.

