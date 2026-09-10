# Claude source contract and Codex gap

All source citations are at Buzz `051c3a270be9c73da9ab06700bcab7d5552fceaa`,
read with `git show` in `/Users/loganj/.buzz/REPOS/buzz-reference`.

- `desktop/src-tauri/src/managed_agents/discovery/catalog.rs:50–84`: Claude CLI
  `claude`, adapter `claude-agent-acp` (legacy `claude-code-acp` alias), install
  hints, provider locked, unstable model switching/native config disabled,
  `~/.claude/settings.json`, CLI auth/status guidance. Executable discovery alone
  is not authentication. This implementation requires the newer reported package
  identity instead of promising native-profile parity for legacy adapters.
- Same file `:85–119`: Codex CLI `codex`, adapter `codex-acp`, config
  `~/.codex/config.toml`, `codex login`/`codex login status`, no model/provider env
  mapping, no unstable switching/native config. These are distinct contracts.
- `crates/buzz-acp/src/config.rs:790–795`: Claude/Codex adapters have zero args,
  unlike Goose `acp`. `:825–865` describes Codex's adapter-specific CODEX_CONFIG
  network override, not a generic provider connection or model proof.
- `crates/buzz-acp/README.md:83–99`: Claude adapter installation and
  ANTHROPIC_API_KEY example. `:67–81`: Codex OPENAI_API_KEY/adapter fallback note;
  do not infer Claude login or Goose/Buzz OAuth-cache reuse from it.
- `desktop/src-tauri/src/managed_agents/claude_config/mod.rs:1–27`:
  ANTHROPIC_MODEL is the sole Claude startup authority; remove BUZZ_ACP_MODEL.
- `desktop/src-tauri/src/managed_agents/runtime.rs:384–390`: explicit
  CLAUDE_CODE_EXECUTABLE vendor executable selection (not arbitrary remote argv).
- `crates/buzz-acp/src/pool.rs:287–320`: exact new Claude package identity is a
  source-grounded native prompt capability gate; Claude uses meta append, Goose
  has its own extension, other agents need protocol >=2 for native field support.
- `crates/buzz-acp/src/acp.rs:636–668,2132–2144`: native Claude session/new
  `_meta.systemPrompt: {append: text}`. `:192–201,366–370`: Codex may return `{}`
  for unknown extensions; success probing must never manufacture capability.
- `desktop/src-tauri/src/managed_agents/readiness.rs:407–462`: vendor-specific
  auth probes differ from static prerequisites; no probe is performed here.

- `crates/buzz-acp/src/acp.rs:2178–2182,2207–2212,2825–2846`: optional
  session/new models.currentModelId/availableModels state and fresh-session
  authority. This is reported model state, not guaranteed by every adapter.

## Deliberate stricter scope

Approved model IDs are local policy, not authenticated discovery. Claude's actual
new-session models report supplies currentModelId/availableModels; the host accepts
only exact matching values, with no fabricated set_model ACK. Unknown reports fail
closed. This is adapter-reported session evidence, not provider attestation. Native
model/config switching is disabled per catalog; optional-config semantics of other
harnesses are untouched. API key files are a new Beehive local transport policy,
not a claimed upstream file format. It reads only explicitly provisioned paths with
service-owner and 0600-style permission checks; no existing owner caches are read.
Private inputs stay local, included in prepared-input authority hashing; vendor CLI
binary hash is also included. Existing trusted-operator spawn TOCTOU limitations
remain, not an OS sandbox or malicious-local-user defense.

## Actual acceptance and remaining work

`test/normal-conversation.test.ts` reuses the existing NORMAL_A661/RECOVERY_5360
actual wizard/service-subprocess/TUI journey for initial normal and selected-B normal
conversion. Claude fixture differs in argv, closed env, exact package initialize,
models.currentModelId report, meta profile transport, rejection of Goose/native
config/set_model, and failure modes. Common CLI tool/reply machinery is deliberately
shared, not reimplemented. Each installed journey requires four real signed Buzz CLI
replies (two channels, later turn and fresh Restart), same identity, native profile,
exact model, unchanged sibling/history/manifest, truthful preflight failures and Stop.
`test/claude.test.ts` exercises private-env/credential validation and actual owned
ACP prerequisite/drift/negative capability paths. Fresh fixture-only keys/loopback.

REAL: `/Applications/Buzz.app/Contents/MacOS/{buzz-acp,buzz}` (hashes in CHECKPOINT).
FIXTURE: TypeScript ACP adapter, Node vendor-CLI placeholder; no vendor login/status
is invoked. SOURCE-ONLY: real Claude API key/provider acceptance; subscription auth;
Codex integration, model/config/profile proof and installed wizard acceptance.

Codex still needs its own closed local auth/env/config contract, source-supported
model selection/reporting and profile-capability handling through these same owners,
then source-shaped ACP fixtures and actual normal wizard/installed conversations.
No source-backed universal model env or Goose-native methods should be invented.
Other providers, presets/custom, relay mesh, compute deployment, production admission,
current kind40002 and live service/user authentication remain explicitly unproved.
