# Agent configuration instructions

Scope: agent configuration surfaces, their shared field model, and the managed
agent runtime catalog. Read the root [`AGENTS.md`](../../../../AGENTS.md) and
[`desktop/AGENTS.md`](../../../AGENTS.md) first.

## Sources of truth

The Rust `KNOWN_ACP_RUNTIMES` catalog in
[`discovery.rs`](../../../src-tauri/src/managed_agents/discovery.rs) is canonical
for harness capability facts, including model, provider, and effort support and
application keys. Its `KnownAcpRuntime` type lives in
[`runtime_metadata.rs`](../../../src-tauri/src/managed_agents/discovery/runtime_metadata.rs).
Add a capability to the catalog, expose it through `AcpRuntimeCatalogEntry`, then project it in
[`lib/agentConfigCore.ts`](lib/agentConfigCore.ts). Do not create a competing
TypeScript capability table or render-time harness-ID checks.

TypeScript owns presentation and persistence policy. Keep field visibility,
omission reasons, and dependent-value clearing in the named field model and
policy types. Persist effort through the descriptor’s `currentPersistence`;
apply it through `targetApplication`. A missing runtime catalog entry means
metadata is unknown, not that the harness lacks a feature, so surfaces must show
their loading or error state instead of silently hiding fields.

## Coverage

When changing this behavior, update the focused tests:

- `lib/agentConfigCore.test.mjs` for field and clearing policy;
- `ui/agentConfigFieldsContract.test.mjs` for rendering/disclosure behavior;
- `ui/usePersonaModelDiscovery.test.mjs` for model discovery status and cache
  behavior; and
- `desktop/tests/e2e/onboarding-agent-defaults.spec.ts` for onboarding flows
  when affected.

Also run the relevant Rust runtime metadata tests and the desktop checks named
in [`desktop/AGENTS.md`](../../../AGENTS.md).
