# triage-service

External backend for the Buzz `/triage` proof of concept. All triage logic,
todo storage, and learned corrections live here — the Buzz desktop app only
collects unread messages, posts them here, and renders the response.

It runs as a standalone process and shares nothing with the relay. Only
`data.json` (its runtime state) is gitignored.

## Run

No dependencies. Requires Node 18+ (Node 24 ships with the repo's Hermit
toolchain).

```bash
cd triage-service
node server.mjs          # http://localhost:8787
```

Point the desktop app at it with `VITE_TRIAGE_API_URL` if you change the port.

## Classification modes

By default it uses a transparent heuristic scorer (DMs and mentions score up,
short acknowledgements and un-addressed channel chatter score down), which
makes the PoC demoable without an API key.

For LLM classification:

```bash
TRIAGE_LLM=1 OPENAI_API_KEY=sk-... node server.mjs
# optional: TRIAGE_MODEL=gpt-4o-mini
```

If the LLM call fails, each item falls back to its heuristic verdict, so a scan
never returns an empty result.

## Learning loop

Every user action in the UI posts to `/feedback`. Those rows are aggregated by
`buildLessons` in [classify.mjs](classify.mjs) into per-thread, per-author, and
per-channel weights that shift the next scan's scores, and the most recent
corrections are injected into the LLM prompt as examples. Thread-level
corrections carry the most weight, channel-level the least.

Promoting an item out of Filtered is the strongest signal available — it says
the agent was wrong in the direction that matters.

## Endpoints

- `POST /scan` — `{ pubkey, candidates }` returns `{ suggestions }`
- `GET /suggestions?pubkey=` — last scan result
- `GET /todos?pubkey=`
- `POST /todos` — adopt a suggestion
- `PATCH /todos/:id` — `{ pubkey, status: "done" | "dismissed" | "open" }`
- `POST /feedback` — `{ pubkey, eventId, suggestedVerdict, userAction }`
- `GET /health`

State persists to `data.json` beside the server.
