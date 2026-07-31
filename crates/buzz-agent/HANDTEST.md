# Hand-testing `buzz-agent`

This is a **swap**, not a new thing. `buzz-agent` keeps its name, its binary,
its ACP contract and its place in the workspace — only the internals changed
from a hand-written agent loop to the `goose` crate used as a Rust library.

So there is nothing special to run. The normal flow *is* the test:

```bash
just dev            # the whole app
# or
just relay          # terminal 1
just goose          # terminal 2 — agent against that relay
```

Both already build and use the swapped `buzz-agent`. If you see a difference,
that is the bug.

## What to look at

Ordered by risk. The automated suite (41 tests) covers each of these at the
stdio layer; this list is the part only a human can judge.

### 1. Persona actually arrives

```
@fizz who are you?
```

Should answer *as Fizz* and know the `buzz` CLI exists. A generic goose answer
means the system prompt was dropped — the exact failure that makes plain-ACP
embedding impossible, and the reason this uses the library API instead.

### 2. `_Stop` veto

```
@fizz make a todo list with 3 items, then stop immediately without doing them
```

Must refuse to stop while items are open. Capped at 3 vetoes, so it ends
eventually regardless. Look for `_Stop hook vetoed end of turn` in the log.

### 3. Streaming feel

The old loop emitted one chunk per round; goose streams token-by-token. Tests
prove the relay is not write-amplified (chunks are coalesced by identity key,
flushed at 500ms, paced at 167ms/90-per-minute), but **only a human can say
whether it feels better or worse in the desktop app.** This is the most likely
source of "something is off".

### 4. Cancel mid-tool

Ask for something long (`count slowly to 100 with a shell sleep`), then stop.
The turn should end promptly with **no tool call left spinning**. Cancellation
is a cooperative drain with a 5s budget — dropping the stream instead would
leave the MCP child running and the spinner stuck forever. That bug existed and
is fixed; this is the visual confirmation.

### 5. Steering

Send a second message while the agent is working. It should be absorbed into
the running turn, not cancel-and-restart it.

### 6. Model picker

Should list models, not just the current one. An absent catalog is degraded UX,
never a session failure.

### 7. Tool hygiene — known deviation

The model can now *see* `_Stop` and `_PostCompact`: goose's allowlist gates
advertising and dispatch through the same cache, so hiding them would also make
them undispatchable and break the veto. A system-prompt extension tells the
model to leave them alone. Watch for it calling `_Stop` itself — if that
happens the guidance needs strengthening.

## Comparing against the old behaviour

The old loop is gone, so A/B means checking out `main` in a second worktree and
running that relay side by side.

## Known gaps

* **Never run against a real provider.** All automated coverage uses a fake SSE
  server. Databricks *chat* now goes through goose entirely and is unexercised
  — though Databricks *model discovery* still uses our own code, moved to
  `buzz-model-catalog` (the desktop cannot link goose: native `sqlite3`
  collision with its own rusqlite).
* **Relay-mesh MoA is restored but only tested against a fake router.** With
  `mesh-llm` running, a relay-mesh agent on `auto` should switch to the virtual
  `mesh` model within ~2 turns once ≥2 models are live, and fall back to `auto`
  within 30s of the mesh shrinking. Grep the agent log for
  `relay-mesh auto:`.
* Binary is roughly +22.7 MiB raw / +6.0 MiB gzip.
* Nothing changed in packaging or the harness catalog — same sidecar name, so
  nothing needed to.
