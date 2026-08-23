# First-Class Ollama and Hugging Face Support

## Summary

Baseline: Buzz mainline integration at `b50350ca`.

- Ollama already works through `openai-compat`, but requires manual endpoint,
  model, and placeholder API-key configuration. It is documented, not
  first-class.
- Hugging Face Hub models already work as manually entered Buzz Mesh model
  references. Hugging Face hosted inference and Hub search are not first-class.
- Add `ollama` and `huggingface` as named Buzz Agent providers. Keep existing
  generic OpenAI-compatible configurations fully compatible.
- Deliver Ollama in three ownership modes: connect only, manage models on an
  external daemon, or fully manage a Buzz-private runtime.
- Deliver Hugging Face hosted inference plus authenticated Hub search for Buzz
  Mesh. Ollama Hub imports remain a later adapter.
- This direction supports Buzz's provider-swap and community-owned compute
  vision. Ollama and Hugging Face both expose the required OpenAI-compatible
  Chat surface; model tool-calling support remains model-dependent.

References:

- [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility)
- [Hugging Face Inference Providers](https://huggingface.co/docs/inference-providers/index)

## Architecture and Interfaces

- Introduce a typed Rust provider-profile catalog as the single authority for
  provider IDs, labels, runtime availability, credentials, base URLs,
  transport dialect, model discovery, token-limit spelling, and default
  capability policy.
  - Expose sanitized profiles through the existing Rust runtime catalog.
  - Remove the equivalent Desktop TypeScript provider/credential tables.
  - Preserve existing providers and aliases while correcting the current
    `openai-compat` readiness inconsistency.
  - Advertise `ollama` and `huggingface` only for `buzz-agent`; Goose, Claude,
    and Codex behavior remains unchanged.
- Keep the existing LLM transport enum. Both new profiles use the OpenAI
  transport adapter:
  - Chat Completions is the default.
  - An explicit existing `OPENAI_COMPAT_API=responses` override remains
    available.
  - Chat requests use `max_tokens`, retain standard tool-call messages, and
    omit reasoning-effort fields unless a future verified provider/model
    capability enables them.
- Add public Buzz Agent configuration:
  - `BUZZ_AGENT_PROVIDER=ollama`, `OLLAMA_BASE_URL` defaulting to
    `http://127.0.0.1:11434/v1`, and `BUZZ_AGENT_MODEL`.
  - `BUZZ_AGENT_PROVIDER=huggingface`, `HF_INFERENCE_BASE_URL` defaulting to
    `https://router.huggingface.co/v1`, `HF_TOKEN`, and `BUZZ_AGENT_MODEL`.
  - Continue accepting the generic `openai` and `openai-compat` paths without
    migration.
- Add generic provider-secret commands that accept only credential IDs declared
  by the Rust catalog:
  - Set, clear, and return status/source without ever returning secret values.
  - Store one device-wide Hugging Face token in the existing OS-keyring blob.
  - Resolution order is an explicitly supplied effective `HF_TOKEN`, then the
    device keyring.
  - Keyring credentials satisfy local Buzz Agent and Hub readiness only; never
    copy them automatically to remote execution providers.
- Add Desktop-facing Ollama types and commands for ownership mode,
  connection/status, installation, process control, installed models, pull
  progress, deletion, and uninstall.
- Add paginated Hugging Face search/result types containing repository ID,
  immutable revision, gated state, license, downloads, GGUF artifacts,
  quantization, and size.

## Implementation Changes

### Ollama

- Add a machine-level Ollama setup surface used by Buzz Agent configuration:
  - `external`: connect and discover models only.
  - `external_managed_models`: connect plus list, pull, and delete through
    Ollama's native API.
  - `managed`: Buzz downloads, starts, monitors, and stops a private Ollama
    runtime and manages its models.
- Detect an existing daemon at the default or configured endpoint using the
  version and model APIs. Import endpoint, ownership mode, and selected models
  only—never copy weights or adopt its model directory.
- Detect matching existing Buzz `openai-compat` agent configurations and offer
  an explicit, per-agent conversion to `ollama`; preserve model and tuning
  values and use the normal stop/save/restart boundary.
- Implement model management using Ollama's supported `/api/tags`, streaming
  `/api/pull`, `/api/show`, and `/api/delete` operations. Surface pull progress
  and model size; require confirmation before deletion.
- Mark models with confirmed `tools` capability and prefer compatible models
  in agent pickers. Unknown/custom models remain selectable with a warning.
- For managed mode:
  - Download official, pinned, checksum-verified artifacts on demand for
    macOS, Windows, and generic Linux x64/arm64.
  - Use versioned app-data directories, bounded downloads, traversal-safe
    extraction, atomic installation, and a Buzz-private model store.
  - Bind to loopback on port `11434`. If a verified external Ollama owns the
    port, offer import; if another process owns it, fail without terminating
    it.
  - Start on explicit request or when a local Ollama-backed agent starts,
    remain alive until Buzz exits, and stop only the child Buzz launched.
  - Pin upgrades to tested Buzz releases; do not auto-install Ollama's latest
    release.
  - Support separate removal of the runtime and private model store, with
    destructive confirmation.
- For custom external endpoints, reject credentials/fragments, never send
  unrelated secrets, warn for non-TLS network endpoints, and never manage or
  stop the external process.
- Block a remote agent from using the Desktop-managed loopback runtime. Remote
  agents require a reachable external URL and credentials/configuration
  supplied by their execution backend.

References:

- [Ollama model listing](https://docs.ollama.com/api/tags)
- [Ollama model deletion](https://docs.ollama.com/api/delete)
- [Official Ollama releases](https://github.com/ollama/ollama/releases)

### Hugging Face

- Add the named hosted provider using authenticated `/v1/models` discovery and
  `/v1/chat/completions`.
  - Preserve full model IDs and routing suffixes.
  - Map 401/403/429 responses to actionable, redacted errors.
  - Explain that the fine-grained token needs gated-repository read access and
    Inference Providers permission when both features are used.
- Add a reusable backend-owned Hub browser to the Mesh compute model picker:
  - Debounce and paginate server-side searches for text-generation GGUF
    repositories.
  - Use authenticated requests when a keyring/environment token exists, with
    bounded timeouts and bodies and no off-origin authorization redirects.
  - Show gated/private status, license, size, and quantization choices.
  - Convert selection into MeshLLM's immutable raw-GGUF reference,
    `{repo}@{commit}/{artifact}`, and let MeshLLM remain the sole
    resolver/downloader. (`hf://` is reserved for MeshLLM package references.)
  - For inaccessible gated repositories, show a link to request/accept access
    on Hugging Face; never attempt to grant access.
- Coordinate a small MeshLLM SDK change:
  - Add an optional in-memory HF token to the serving builder/config and pass
    it into the model repository.
  - Keep it out of serialization, status payloads, debug output, and logs.
  - Buzz's frontend request remains token-free; the Tauri backend loads the
    keyring immediately before starting/restoring Mesh.
  - Land and pin the SDK change before enabling gated Mesh downloads.
- Updating the stored token restarts affected local hosted-inference agents
  through the existing configuration restart path. A running Mesh node keeps
  serving; the new token applies to its next start/download.

Reference:

- [Hugging Face Hub API](https://huggingface.co/docs/huggingface_hub/en/package_reference/hf_api)

## Implemented Safety Boundaries

The feature branch implements the provider catalog, named transports, Desktop
credential flow, Ollama connection/model lifecycle, and Hugging Face Hub
browser. Two rollout gates intentionally remain closed:

- The managed Ollama installer is present but its checked-in artifact manifest
  is empty. Release engineering must supply a tested version, official URLs,
  SHA-256 values, and independent download/extraction limits before Desktop
  enables installation. External Ollama connection and model management remain
  available.
- The pinned MeshLLM `v0.75.1` can read `HF_TOKEN` only from the process
  environment. Public Hub models and gated models with a launch-time token are
  supported; keyring-only gated selection stays disabled until MeshLLM exposes
  an in-memory token API. Buzz never copies the keyring token into process
  environment as a workaround.

Runtime/model-store uninstall and app-launch Ollama auto-start remain follow-up
work. Managed Ollama does start on explicit request or local agent demand and
stops only the child Buzz launched.

## Test Plan

- Provider catalog contract tests:
  - Rust catalog and Desktop projections agree.
  - Existing provider behavior is unchanged.
  - New providers appear only for Buzz Agent.
  - Credentials, readiness, aliases, base URLs, model keys, and
    provider-switch clearing are consistent.
- Buzz Agent mock-server tests:
  - Ollama and Hugging Face Chat bodies use `max_tokens`.
  - Text replies, single/parallel tool calls, tool results, malformed
    responses, cancellation, timeouts, 401/403/429 handling, and secret
    redaction work.
  - Explicit Responses override remains functional.
- Desktop backend tests:
  - Keyring status/set/replace/delete and environment precedence.
  - No secret crosses IPC or appears in logs/errors.
  - Ollama endpoint validation, daemon detection, import, ownership boundaries,
    pull-stream parsing, deletion confirmation, crash recovery, and port
    conflicts.
  - Installer platform mapping, size/hash checks, safe extraction, atomic
    rollback, and managed-child-only shutdown.
  - Hub pagination, filtering, gated errors, artifact selection, immutable
    refs, response limits, and redirect policy.
- UI and E2E tests:
  - Onboarding/default/create/edit surfaces retain their existing config
    contracts.
  - Ollama's three modes, import flow, progress, model selection, and
    destructive confirmations.
  - Hugging Face keyring state, hosted model discovery, Hub search,
    gated-access guidance, and Mesh download progress.
- Live acceptance:
  - Run a Buzz Agent tool-call round trip against a tool-capable Ollama model.
  - Run the same workflow through Hugging Face hosted inference.
  - Load and serve one public and one authorized gated GGUF through Buzz Mesh.
  - Exercise managed installation and shutdown on macOS, Windows, Linux x64,
    and Linux arm64 release runners or physical test hosts.
  - Run `just ci`; run the relevant real Desktop workflows in addition to mock
    tests.

## Assumptions and Rollout

- Initial scope is Buzz Agent plus Desktop. No relay protocol, event-kind,
  database, mobile, Goose, Claude, or Codex changes.
- Hugging Face Hub installation targets Buzz Mesh only. The browser and result
  types remain adapter-ready for later Ollama GGUF import.
- Managed Linux uses the official generic artifact; ROCm, JetPack, and other
  optional accelerator bundles are deferred. Buzz does not install GPU
  drivers.
- Ollama cloud sign-in is not managed; an externally authenticated daemon may
  still expose cloud models.
- Sequence delivery as independently reviewable changes:
  1. Behavior-preserving Rust provider catalog and Desktop projection.
  2. Named Ollama/Hugging Face transports and keyring-backed HF credentials.
  3. Ollama connection, model-management, and managed-runtime modes.
  4. MeshLLM token API/pin update and Hugging Face Hub browser.
- Update the agent configuration contributor guide, Buzz Agent documentation,
  security documentation, and environment example alongside the corresponding
  interface changes.
