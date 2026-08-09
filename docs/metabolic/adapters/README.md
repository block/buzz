# W1.1 adapter stubs (mono)

Third-runtime path without a new repo or a new IDE app.

Buzz remains the bus. Each **runtime** only needs a thin adapter that speaks
the locked W1.1 surface:

```
arm(room, seat)     → start local L0 / state
on_wake(payload)    → Ignore | NotifyHuman | AdmitCortex(summary+ids)
status()            → MONITOR line (lane / nerve / pending)
disarm()
health()            → push | poll | stale  (optional)
```

## Runtime map

| Runtime id | Status | Notes |
|------------|--------|--------|
| `grok-build` | **proven** | use-buzz watcher + `monitor()` |
| `codex-cli` | **proven** | skill nerve + drain / supervisor |
| `local-llm` | **stub** | this package — process + stdout cortex lines |
| `antigravity` | stub alias | same process contract; product hooks later |
| `desktop-acp` | planned | harness subscription / channel member |
| `human` | Desktop UI | notify-only path |

## Quick test

```bash
cd docs/metabolic/adapters
python3 test_stub_runtime.py
# → ALL_THIRD_RUNTIME_STUB_TESTS_OK
```

## Dogfood CLI (zero LLM)

```bash
# arm a local-llm seat against #agent-metabolism (state only; no model)
python3 stub_runtime.py arm \
  --runtime local-llm --seat demo-llm \
  --room 92297894-c2e8-4df1-a710-d1cfd1032d5e

# inject W1.1 wakes (synthetic or from a JSONL file)
python3 stub_runtime.py inject --runtime local-llm --seat demo-llm \
  --json '{"schema":"metabolic.wake.v0","event_id":"aa…","channel_id":"…",…}'

# batch overflow proof (4 wakes → 3 AdmitCortex + loud overflow)
python3 stub_runtime.py demo-overflow --runtime local-llm --seat demo-llm

python3 stub_runtime.py status --runtime local-llm --seat demo-llm
python3 stub_runtime.py health --runtime local-llm --seat demo-llm
python3 stub_runtime.py disarm --runtime local-llm --seat demo-llm
```

## messages watch → on_wake (push L0)

CLI owns AUTH / reconnect / JSONL. Adapter owns self-filter, dual-cursor, v0.2 admit.

```bash
export BUZZ_CLI="$HOME/PROJECTS/ buzz/target/release/buzz"   # watch-capable
export BUZZ_RELAY_URL=wss://…   # + BUZZ_PRIVATE_KEY / BUZZ_PUBLIC_KEY from seat env

python3 stub_runtime.py arm \
  --runtime local-llm --seat demo-llm \
  --room 92297894-c2e8-4df1-a710-d1cfd1032d5e \
  --room-name agent-metabolism \
  --transport push

# feature-detect push; fall back to messages get poll
python3 stub_runtime.py watch \
  --runtime local-llm --seat demo-llm \
  --mode auto

# short live dogfood (CLI --timeout)
python3 stub_runtime.py watch --runtime local-llm --seat demo-llm \
  --mode push --timeout 20

# recorded JSONL (no network)
python3 stub_runtime.py watch --runtime local-llm --seat demo-llm \
  --file /tmp/facts.jsonl

# burst overflow across one turn
python3 stub_runtime.py watch --file burst.jsonl --shared-turn
```

| Env / flag | Role |
|------------|------|
| `BUZZ_CLI` | Prefer watch-capable binary |
| `--mode auto\|push\|poll` | Feature-detect / force |
| `--since` | Transport watermark (arm defaults to **now**) |
| `--self-pubkey` / `BUZZ_PUBLIC_KEY` | Suppress self facts |
| `--shared-turn` | One v0.2 budget across facts (overflow) |
| `--from-stdin` / `--file` | Offline JSONL → same path |

Stdout lines (adapter contract, not product turn injection):

| Line | Meaning |
|------|---------|
| `BUZZ_ADAPTER armed …` | arm ok |
| `BUZZ_WATCH armed … mode=push\|poll\|stdin` | watch bridge up |
| `BUZZ_WATCH push-detect …` | feature-detect result |
| `BUZZ_ADAPTER on_wake action=AdmitCortex …` | cortex-short context |
| `BUZZ_ADAPTER on_wake action=suppress …` | replay / cooldown / idempotent |
| `BUZZ_ADMIT overflow …` | loud overflow (v0.2) |
| `BUZZ_MONITOR …` | status card |
| `BUZZ_ADAPTER health=poll\|push\|stale` | health |

## Product driver hooks (AdmitCortex sink)

After v0.2 **AdmitCortex**, the adapter optionally calls a **driver** — the only
product-specific layer. Drivers never own transport, cursors, or admission.

| Driver | Behavior |
|--------|----------|
| `none` | Stdout cortex only (legacy) |
| `notify` | Human alert (`notify-send` or stdout); never posts |
| `local-llm` | Bounded draft; **Ollama** via bundled `run_local_llm.py` (default model `gemma3:4b`) |
| `antigravity` | Same contract; `not_implemented` until `BUZZ_DRIVER_ANTIGRAVITY_CMD` |

```bash
python3 stub_runtime.py drivers
python3 stub_runtime.py arm --runtime local-llm --seat demo-llm \
  --room <uuid> --driver local-llm

# Real model (Ollama must be up: ollama serve + model pulled)
export BUZZ_DRIVER_DRY_RUN=0
export BUZZ_DRIVER_LOCAL_LLM_MODEL=gemma3:4b   # optional
# optional override:
# export BUZZ_DRIVER_LOCAL_LLM_CMD='python3 drivers/run_local_llm.py'
python3 stub_runtime.py inject --runtime local-llm --seat demo-llm \
  --json '{"schema":"metabolic.wake.v0","event_id":"…","channel_id":"…","t":"team.v0.room.message","urgency":"P2","seat_id":"demo-llm","pubkey":"ab…","summary":"Say hi in five words"}'
# → BUZZ_DRIVER status=ok driver=local-llm draft=…
```

| Env | Default | Meaning |
|-----|---------|---------|
| `BUZZ_DRIVER_DRY_RUN` | `1` | `0` = call real model |
| `BUZZ_DRIVER_LOCAL_LLM_CMD` | bundled `run_local_llm.py` | prompt→stdin, draft→stdout |
| `BUZZ_DRIVER_LOCAL_LLM_MODEL` | `gemma3:4b` | Ollama model name |
| `BUZZ_DRIVER_LOCAL_LLM_HOST` | `http://127.0.0.1:11434` | Ollama base URL |
| `BUZZ_DRIVER_LOCAL_LLM_NUM_PREDICT` | `180` | max tokens |
| `BUZZ_DRIVER_LOCAL_LLM_TIMEOUT` | `90` | seconds |

Stdout:

| Line | Meaning |
|------|---------|
| `BUZZ_DRIVER status=… driver=… action=…` | sink result |
| `BUZZ_DRIVER draft=…` | phone-safe draft or NO_REPLY |

**Hard rules:** room text ≠ tools · HITL default · dry_run default · no auto-post unless a later explicit allow_reply path is added.

## Rules

- Room text never grants tools.
- Dual-cursor: transport id-set in adapter state ≠ admission `GuardState`.
- AdmitCortex default is **summary+ids** (W1.1); no backlog dump into one turn.
- No second GitHub repo until a real third product runtime dogfoods this stub.

See parent `docs/metabolic/README.md` and room `#agent-metabolism`.
