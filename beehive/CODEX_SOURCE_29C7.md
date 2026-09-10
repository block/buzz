# Codex closed API-key subset

Source contract is Buzz `051c3a270be9c73da9ab06700bcab7d5552fceaa`, not a
claim that the installed executables were built from that source:

- `desktop/src-tauri/src/managed_agents/discovery/catalog.rs:85–119`:
  separate codex/codex-acp executables, TOML config, operator login/status guidance;
  no model-env mapping or native switch.
- `config_bridge/codex.rs:125–130` in that same managed_agents directory:
  CODEX_HOME/config.toml, not CODEX_CONFIG as a path.
- `crates/buzz-acp/src/config.rs:825–832`, `acp.rs:244–271`:
  CODEX_CONFIG JSON session overrides. Beehive provides only `{model}` to its
  independently owned adapter; it does not inherit parent config or network widening.
- `crates/buzz-acp/README.md:67–81`: explicit OPENAI_API_KEY fallback, not
  ChatGPT subscription parity. This subset requires a local owner-only key file;
  it neither reads nor imports login caches. Key presence is not authenticated.
- `pool.rs:295–320,5505–5508`: native bare systemPrompt requires protocol >=2
  for Codex; no Claude append or Goose extension. Both Beehive ACP boundaries
  request and require version 2 and the codex-acp identity. Unknown names/versions
  are deliberately unsupported, not an inferred universal adapter capability.
- `acp.rs:2178–2182`: optional fresh-session currentModelId/availableModels.
  Missing/mismatched evidence rejects before prompts; no invented switch ACK.

Initial wizard option 5 offers normal (default) or explicit diagnostic setup.
Local add-codex and selected-B normal conversion create immutable references;
models/workspaces remain operator approved, keys/profiles independent. Normal
runtime/tool/relay/trust remain installation-common. Local prerequisite validation
and manifest write share host.lock; Start rereads prerequisites. Stop does not.

Fixtures are TypeScript Codex-shaped ACP, not actual Codex. The vendor executable
is explicitly a Node placeholder. Native protocol2/systemPrompt, closed key/config
and HOME distinctions, missing/wrong model/protocol/native identity/auth and later
drift are exercised. Normal acceptance uses actual wizard -> host CLI subprocess ->
TUI -> read-only installed buzz-acp/Buzz CLI, four signed same-identity replies each
(initial and selected-B conversion), fresh Restart, native profile/exact model,
old definitions/history and unchanged Y. Actual local key removal and service reopen
reject Start/Restart before another adapter launch; Stop does not recreate keys.

Self-review added an explicit diagnostic model-proof flag: a caller cannot catch a
failed catalog and then prompt the partially created Codex session. No optional
settings/load/resume, custom provider assurance, subscription/login-cache parity,
real provider inference/authentication, production/current-kind40002, remote
provisioning, deployment or repository-wide CI is claimed. Protocol/report support
is conditional and fails closed. Existing NORMAL_REVIEW_425FEB34 and
CLI_LIFECYCLE_REVIEW_256CDF31 are consumed as CLOSED scoped no-blocker reports;
this Codex delta has author self-review, not independent approval. Historical
broker/tool failures remain unclassified by any green run.

Evidence logs and separate source/binary hashes:
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/CODEX_29C7/` (workspace, outside repository).
