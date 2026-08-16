# Gemma 4 26B 64K local admission

Command Adviser admits one exact local inference runtime for disconnected use:

| Property | Admitted value |
| --- | --- |
| Model | `google/gemma-4-26b-a4b` |
| Loaded instance | `gemma4-26b-official` |
| Context | 65,536 tokens |
| Maximum generated output | 8,192 tokens |
| Reasoning | `off` |
| Generation capacity | one request at a time |
| Required capabilities | native image input and trained tool use |

The desktop reads the live LM Studio catalog and reports ready only when the
model, instance, capabilities, loaded context, and parallelism all match. A
catalog model's advertised maximum context does not satisfy admission by
itself. The loaded instance must be configured at 64K with parallelism one.

Every managed Buzz LM Studio adviser receives the same 64K/8K/reasoning-off
runtime projection. Multiple advisers may prepare retrieval and deterministic
work concurrently, while LM Studio serialises generation through the single
resident model. The extended 900-second timeout allows a queued adviser to wait
for its turn or complete a long-context prompt.

## Skills, memory, and tools

Selected skill bodies are preloaded into the native system prompt with a 32
KiB per-skill and 64 KiB aggregate limit. This preserves existing skills on a
transport that cannot call Buzz's legacy stdio `load_skill` tool. It does not
yet implement autonomous skill authoring or revision.

Memory and RAG remain external to model weights. They are available through
explicitly allowlisted, literal-loopback HTTP MCP integrations after their
MacBook-local copies are commissioned. Historical memory can therefore remain
complete while retrieval selects recent and task-relevant material for the
current 64K prompt.

JPEG, PNG, and WebP ACP image blocks are accepted. Images are base64-validated,
limited to 3 MiB each, and translated to the installed LM Studio native chat
wire format.

## Operator checks

Load the model in LM Studio with identifier `gemma4-26b-official`, context
65,536, and parallelism one. Keep the server on `127.0.0.1:1234`.

Run the exact catalog and reasoning-off generation canary:

```bash
just check-offline-model test-results/offline-model/gemma64-adapter.json
```

After building the release adapter, exercise the real ACP text and image path:

```bash
python3 scripts/live-lmstudio-adapter-canary.py \
  --binary target/release/buzz-lmstudio-agent \
  --image desktop/public/landing/buzz-wordmark.png \
  --cwd "$PWD"
```

The checker is read-only with respect to model configuration: it never loads,
unloads, or downloads a model. Both checks fail closed if the exact qualified
runtime is absent or emits reasoning.

## Qualification boundary

The 64K tier passed exact text, strict JSON, reasoning-off, stateful
continuation, structured tool-call, native-image, cancellation/recovery,
three-request queue, and 62,080-token retrieval tests on the 64 GB M5 Pro.
The 128K tier is not admitted because its observed time-to-first-response was
not operationally acceptable. A physical restart and acceptance run with all
external networking unavailable remains required before declaring the whole
Command Adviser stack disconnected-ready.
