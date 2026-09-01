# buzz-accumulator

Fold engine + standalone daemon: mirrors everything your key can see from the
relay into local SQLite, and folds selections of that stream into small,
always-current, provenance-carrying artifacts.

## The three nouns

Signals — raw relay events, never rewritten — are the substrate. On top of
them:

- **Selection** — a frozen-or-live description of a signal set: who × what ×
  when (`channels`, `authors`, `kinds`, plus the selection's own `since` /
  `until_exclusive`). A pinned `until_exclusive` **freezes** it; an open end
  means **live** — "and whatever comes next".
- **Fold** — name + selection + model + instructions: a **factory for
  artifacts**. Each run computes `artifact' = fold(artifact, new_signals)`.
  A frozen selection makes the fold run until its set is covered, then it is
  done forever (every further preflight says *cached*). A live fold is never
  done — the prior version rides along and new signals keep folding in.
- **Artifact** — the model's response, **verbatim**, persisted as an
  immutable append-only version chain. Provenance (`shown_ids`, coverage
  window, model, prompt hash) is engine-computed from what the model was
  actually shown — never parsed out of model output.

There is no rollup machinery: **publish** an artifact back into a channel and
it becomes a signal again, so a later fold can simply select it. Folds all the
way down.

Coverage is **per fold**: the same events can feed any number of folds (one
summarizes yesterday, another counts how often "Tua" was said). Within one
fold's chain, an event folds in exactly once. Note the corollary: editing a
fold's instructions does not re-fold already-covered events — a new lens is a
new fold.

## Honesty invariants

- Preflight is $0: plans and estimates are computed without any model call;
  an unknown model's window fit is an honest `null`, never a guess.
- Coverage records exactly the signals the model was shown. An oversized
  window truncates honestly and the remainder stays pending; unread signals
  are never sealed as covered.
- Runs are pinned to their priced window: the optional request clamp can only
  **narrow** the selection's own window, so what you priced is what you run.
- A frozen selection never reads outside its freeze, no matter what clamp the
  caller sends.
- The output is free-form; `[event:<id>]` citations in it are
  reader-verifiable links (resolve via `GET /events/{id}`), not a validated
  contract.

## Running it

```sh
# daemon (loopback HTTP on 127.0.0.1:4640; mirror in ~/.buzz-accumulator/)
BUZZ_PRIVATE_KEY=<nsec-or-hex> cargo run -p buzz-accumulator

# lab UI (its own pnpm workspace; proxies /api to the daemon)
cd crates/buzz-accumulator/ui && pnpm install && pnpm dev   # → http://localhost:5173
```

The relay is pinned to the team relay by default; the generic `BUZZ_RELAY_URL`
env var is deliberately ignored (override with `--relay` /
`BUZZ_ACCUMULATOR_RELAY_URL`). Useful overrides: `BUZZ_ACCUMULATOR_DB`,
`BUZZ_ACCUMULATOR_HTTP_ADDR`.

The HTTP API is **loopback-only and unauthenticated by design** —
`POST /folds/{name}/run` spends real money and `publish` posts real messages,
so any non-loopback bind must add authentication first.

## HTTP API

| Route | What |
|---|---|
| `GET /status` | Connection, backfill, mirror counts. |
| `GET /channels` | Discovered channels. |
| `GET /events/{id}` | One mirrored event (the citation-chip endpoint). |
| `POST /select/preview` | `{selection, since?, until_exclusive?}` → count, size, daily rhythm. $0. |
| `POST /select/events` | Same body + `limit`, `after` keyset cursor → the actual events, paged. |
| `GET /folds` | All fold specs. |
| `PUT /folds/{name}` | `{selection, model, instructions?, meta?}` — create/update a fold. |
| `GET /folds/{name}` | Spec + chain length + latest artifact summary. |
| `DELETE /folds/{name}` | Delete the spec (artifacts are history and survive). |
| `POST /folds/{name}/preflight` | `{since?, until_exclusive?, include_input?}` → cached / stalled / ready with exact cost estimate and (opt-in) the exact model input. $0. |
| `POST /folds/{name}/run` | Same window clamp → runs the model, appends one artifact version. **Spends money.** |
| `GET /folds/{name}/artifacts` | The version chain (summaries). |
| `GET /folds/{name}/artifacts/{version}` | One full artifact with provenance. |
| `POST /folds/{name}/artifacts/{version}/publish` | `{channel, allow_cross_channel?}` → posts the artifact into the channel as a message. Guarded: refuses unless the artifact's chain has only ever read that channel, unless crossed deliberately. |

`since` / `until_exclusive` on preview/preflight/run are a **clamp**: the
selection's own window is authoritative and the clamp can only narrow it.

## Layout

- `src/` — pure engine (selection, spec, plan/complete run, estimate,
  transcript). No relay I/O, no storage.
- `src/daemon/` — the standalone app (`daemon` feature, on by default):
  relay sync → SQLite mirror → HTTP API → relay publish. Engine consumers
  depend with `default-features = false`.
- `ui/` — the vite + React lab client. Deliberately its own pnpm workspace
  root, invisible to the repo's `just ci`.
