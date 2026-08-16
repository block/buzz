# Gemma 4 Phase 1 online acceptance — 16 August 2026

## Acceptance boundary

This is the online-first Phase 1 acceptance checkpoint. Internet and LAN
connectivity remained enabled, but Command Adviser inference was routed to the
MacBook-local LM Studio runtime. Physical disconnected acceptance belongs to
Phase 5 and is not claimed here.

## Admitted runtime

- Model key: `google/gemma-4-26b-a4b`
- Loaded instance: `gemma4-26b-official`
- LM Studio: `0.4.21` build 2
- Endpoint: `http://127.0.0.1:1234`
- Context: 65,536 tokens
- Maximum output: 8,192 tokens
- Reasoning: off
- Generation capacity: one
- Loaded-instance catalogue before and after the canaries: exactly one instance

Production preflight clamps the legacy schedule concurrency setting to the
qualified one-slot runtime. The generic runtime identity continues to support
one- and two-slot test fixtures so runtime-swap coverage is preserved.

## Live canaries

The following checks passed against the admitted instance:

- Exact text response: `GEMMA64 READY`
- Strict JSON response
- Stateful continuation using `ANCHOR-642`
- Structured function call to `lookup_readiness` with
  `{"system":"command-adviser"}`
- Native image input
- Cancellation followed by successful same-session recovery
- Reasoning-token count of zero
- Three overlapping submissions generated without overlap in FIFO order 1, 2,
  3 through the parallel-one instance
- One-, two-, and three-adviser contributions were consolidated without
  contributor loss or identity mixing
- No second loaded LM Studio instance appeared

The collaboration acceptance ran the production `AdviserExecutor`, immutable
Command Adviser personas, strict output validation, and production
`LocalModelScheduler::sequential()` path for three bounded cases: Operations
alone; Operations with Intelligence; and Operations, Intelligence, and
Logistics. Each case retained its supplied adviser identities, returned the
qualified instance ID, reported zero reasoning tokens, and ran in FIFO order at
capacity one.

LM Studio 0.4.20 exposed two runtime-contract faults during this test. Its
native API rejected the documented custom identifier, and Command Adviser still
sent a stale 32,768-token request context. LM Studio 0.4.21 build 2 accepts the
qualified identifier. Command Adviser now sends the exact loaded 65,536-token
context and temperature zero. Gemma may still wrap otherwise valid JSON in one
`json` Markdown fence, so the product accepts only that single bounded envelope
before applying the unchanged strict schema and citation validation; prose,
nested fences, extra fields, wrong advisers, and unsupported citations remain
rejected.

Reproducible commands:

```bash
. ./bin/activate-hermit
just check-offline-model test-results/offline-model/gemma64-phase1-online-20260816.json
python3 scripts/live-lmstudio-adapter-canary.py \
  --binary target/release/buzz-lmstudio-agent \
  --image desktop/src/assets/command-adviser/hmas-supply-badge.png \
  --cwd "$PWD" \
  > test-results/offline-model/gemma64-adapter-complete-phase1-online-20260816.json
python3 scripts/live-lmstudio-tool-call-canary.py
python3 scripts/live-lmstudio-serial-queue-canary.py \
  > test-results/offline-model/gemma64-serial-queue-phase1-online-20260816.json
BUZZ_LIVE_LMSTUDIO_ACCEPTANCE=1 \
BUZZ_LIVE_LMSTUDIO_ENDPOINT=http://127.0.0.1:1234 \
BUZZ_LIVE_LMSTUDIO_INSTANCE=gemma4-26b-official \
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  one_to_three_advisers_use_the_real_capacity_one_product_path \
  -- --ignored --nocapture
```

The shell regression `scripts/tests/check-offline-model-test.sh` proves that
the live canary sends generation to `gemma4-26b-official`, not to the underlying
model key. This prevents LM Studio from silently auto-loading a second instance.

## Installed-app Daily Command Brief

The installed app completed and published Daily Command Brief run
`e6427652-99f7-484b-8c92-dd7fecbd6ac8` on the local model. The run progressed
through collection, specialists, consolidation, securing, and terminal
publication. The local audit spool records:

- Status: `degraded`
- Publish state: `published`
- Append sequence: 1
- Published at: `2026-08-16T05:07:21Z`

The degradation was limited to unavailable Apple Calendar and Reminders input;
LM Studio was local; the existing trusted-LAN Memory and RAG services were shown
connected. The brief produced usable content and COA A/B decision options.

## Known boundary

LM Studio rejects a dynamically supplied private-LAN MCP URL through the native
chat API. Private MCP servers require LM Studio `mcp.json` configuration and API
authentication. Command Adviser's accepted Phase 1 path does not depend on that
dynamic mechanism: trusted Memory and RAG evidence is collected and bounded by
the Rust source layer before local inference. Direct private-LAN MCP execution
from LM Studio is therefore not claimed at this checkpoint and will be revisited
only if a later phase needs it.

## Remaining Phase 1 gate

The corrected commit must be built, installed with a rollback copy retained,
and confirmed by the owner in the installed application before PR #23 is marked
ready or merged.
