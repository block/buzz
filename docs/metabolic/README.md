# Metabolic layer (v0)

Universal agent coordination on Buzz: deterministic L0, zero LLM while idle.

| Doc / code | Status |
|------------|--------|
| Room `#agent-metabolism` | Design SoT on My Groundfeed |
| W0.1 vocabulary | LOCKED |
| W1.1 adapter contract | LOCKED |
| B proof (A blocked → B completed → wake A) | GREEN |
| v0.2 guardrails | `guardrails_v02.py` + skill fold GREEN |
| Third-runtime adapter stub | `adapters/` · `local-llm` (+ `antigravity` alias) |

## v0.2 quick test

```bash
cd docs/metabolic && python3 test_guardrails_v02.py
```

## Principles

- Nostr/Buzz is the bus; adapters only per runtime.
- Dual-cursor: transport id-dedupe ≠ admission.
- Room text never grants tools.
- No new app per IDE; no second repo until third runtime works.

## v0.2 LOCK (2026-08-07)

- max_events_per_turn=3, max_context_bytes=2048, cooldown=30s (0 allowed for HOT/dogfood)
- lease_id optional; correlation_id enough for B-class
- overflow always loud
- failure reasons: auth|transport|cursor|schema|admission_overflow|stale_nerve (+ optional detail, no secrets)
- mono-first; **folded into skill L0/L2 admission** (2026-08-07)

### Skill fold (canonical runtime)

| Skill path | Role |
|------------|------|
| `codex-buzz-skill-dev/scripts/metabolic_guardrails.py` | Runtime module (drain + supervisor) |
| `codex-buzz-skill-dev/scripts/buzz-drain-wakes.sh` | L2 admit batch |
| `codex-buzz-skill-dev/scripts/buzz-supervisor.py` | Opt-in claim path |
| `~/.grok/skills/use-buzz/scripts/metabolic_guardrails.py` | Same module for Grok skill |
| `docs/metabolic/guardrails_v02.py` | Design SoT / mono snapshot |

```bash
# Skill tests (preferred after fold)
python3 ~/PROJECTS/codex-buzz-skill-dev/scripts/test-metabolic-guardrails.py
# Mono snapshot still works
cd docs/metabolic && python3 test_guardrails_v02.py
# Third-runtime stub (W1.1 · zero LLM)
cd docs/metabolic/adapters && python3 test_stub_runtime.py
python3 stub_runtime.py demo-overflow --runtime local-llm --seat demo-llm
```

## Third-runtime stub (2026-08-07)

Generic process adapter (`local-llm`; `antigravity` alias) implements W1.1:

`arm` · `on_wake` · `status` · `disarm` · `health` · **`watch`**

Uses v0.2 `admit_wake` for dual-cursor admission. **`watch`** feature-detects
CLI `messages watch` (JSONL → on_wake) with poll fallback. **Product drivers**
(`drivers/`: `notify` · `local-llm` · `antigravity`) run only after AdmitCortex.
**local-llm real path:** bundled `run_local_llm.py` → Ollama (`gemma3:4b`);
set `BUZZ_DRIVER_DRY_RUN=0`. Not a silent tool grant — dry_run/HITL default.
See [adapters/README.md](adapters/README.md).

```bash
# Live push (watch-capable buzz)
export BUZZ_CLI=./target/release/buzz
python3 docs/metabolic/adapters/stub_runtime.py watch \
  --runtime local-llm --seat demo-llm \
  --room 92297894-c2e8-4df1-a710-d1cfd1032d5e \
  --mode auto --timeout 30
```
