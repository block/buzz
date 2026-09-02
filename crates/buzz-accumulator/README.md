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
  A fold also carries an explicit `order` policy for when one run cannot hold
  every pending signal: `oldest-first` (default) walks the backlog forward —
  repeated bootstrap runs cover earliest → latest with no holes — while
  `newest-first` keeps the freshest evidence and lets history backfill.
  Overridable per run; the transcript the model sees is chronological either
  way.
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
- The planning budget is **model-aware**: window − reserved output − safety
  margin, with the prior artifact, instructions, and source-id list charged
  before the transcript gets the remainder. A 200k-window model plans runs an
  order of magnitude larger than the old flat ceiling; an unknown model falls
  back to that conservative ceiling instead of guessing. Hard event and prior-
  size guards remain as emergency limits only.
- Every ready preflight names its **limiting constraint** (`none`,
  `token-budget`, `event-cap`) and carries the full budget breakdown plus
  estimate headroom — a boundary is always explainable.
- Every preflight and `GET /folds/{name}` carries `coverage:
  {processed, pending, complete}` — the explicit completion state of a
  multi-pass baseline (`complete` only ever true for a fully covered frozen
  selection).
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
| `PUT /folds/{name}` | `{selection, model, instructions?, order?, meta?}` — create/update a fold. |
| `GET /folds/{name}` | Spec + chain length + latest artifact summary + `coverage {processed, pending, complete}`. |
| `DELETE /folds/{name}` | Delete the spec (artifacts are history and survive). |
| `POST /folds/{name}/preflight` | `{since?, until_exclusive?, order?, include_input?}` → cached / stalled / ready, each with `coverage`; ready adds estimate + budget breakdown + limiting constraint and (opt-in) the exact model input. $0. |
| `POST /folds/{name}/run` | `{since?, until_exclusive?, order?}` → runs the model, appends one artifact version. **Spends money.** |
| `GET /folds/{name}/artifacts` | The version chain (summaries). |
| `GET /folds/{name}/artifacts/{version}` | One full artifact with provenance. |
| `POST /folds/{name}/artifacts/{version}/publish` | `{channel, allow_cross_channel?}` → posts the artifact into the channel as a message. Guarded: refuses unless the artifact's chain has only ever read that channel, unless crossed deliberately. |

`since` / `until_exclusive` on preview/preflight/run are a **clamp**: the
selection's own window is authoritative and the clamp can only narrow it.

### Selection defaults are the client's job

The daemon owns no "channels I follow" or "conversations I participated in"
shorthands — a selection is always explicit channel/author ids. The mirror
makes resolving them trivial for a client: `GET /channels` lists every channel
the key follows (the mirror only ever contains those), and "participated in
since X" is `{authors: [<my pubkey>], since: X}`. Resolve, then create the
fold.

## Layout

- `src/` — pure engine (selection, spec, plan/complete run, estimate,
  transcript). No relay I/O, no storage.
- `src/daemon/` — the standalone app (`daemon` feature, on by default):
  relay sync → SQLite mirror → HTTP API → relay publish. Engine consumers
  depend with `default-features = false`.
- `ui/` — the vite + React lab client. Deliberately its own pnpm workspace
  root, invisible to the repo's `just ci`.
