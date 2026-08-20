# triage-service

External backend for the Buzz Inbox fibre engine. It turns channel and DM
messages into **fibres** — ideas, asks, decisions, commitments, questions,
blockers, and FYIs — and keeps the open set up to date as new messages arrive.

The desktop app posts every new message here. Classification always sees the
current incomplete fibres so a message can create a fibre, attach to one,
merge two, or be skipped.

It runs as a standalone process and shares nothing with the relay. Only
`data.json` (its runtime state) is gitignored.

## Run

No dependencies. Requires Node 18+ (Node 24 ships with the repo's Hermit
toolchain).

```bash
cd triage-service
node server.mjs          # http://localhost:8787
```

Or from the repo root: `./scripts/triage-up.sh`. Point the desktop app at it
with `VITE_TRIAGE_API_URL` if you change the port.

```bash
node --test apply.test.mjs classify.test.mjs
```

## Classification modes

By default it uses a transparent heuristic (mentions, asks, commitments, and
questions become fibres; acknowledgements are skipped; same-thread messages
update an open fibre). That makes the PoC demoable without an API key.

For LLM classification:

```bash
TRIAGE_LLM=1 OPENAI_API_KEY=sk-... node server.mjs
# optional: TRIAGE_MODEL=gpt-4o-mini
```

If the LLM call fails, the batch falls back to the heuristic, so ingest never
returns an empty result for a qualifying message.

## Endpoints

- `POST /ingest` — `{ pubkey, messages }` classifies unseen messages against
  all open fibres and returns `{ fibres, openCount, clearedCount, changes }`
- `GET /fibres?pubkey=` — open fibres, score-desc, plus `clearedCount`
- `PATCH /fibres/:id` — `{ pubkey, status: "done" | "dismissed" | "open" }`
- `POST /fibres/restore` — `{ pubkey }` reopens every done/dismissed fibre
- `POST /feedback` — `{ pubkey, fibreId, userAction, ... }` (done, dismissed,
  delegated)
- `GET /health`

State persists to `data.json` beside the server.
