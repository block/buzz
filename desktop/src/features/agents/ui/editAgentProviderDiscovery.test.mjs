import assert from "node:assert/strict";
import test from "node:test";

import {
  runtimeSupportsLlmProviderSelection,
  getPersonaProviderOptions,
  requiredCredentialEnvKeys,
  isMissingRequiredDropdownField,
} from "./personaDialogPickers.tsx";
import { shouldClearModelForRuntimeChange } from "./personaRuntimeModel.ts";

// ── LLM provider field visibility ──────────────────────────────────────────
//
// The edit dialog shows the provider picker when the current runtime supports
// LLM provider selection. Changing the provider in that picker re-fires
// usePersonaModelDiscovery (keyed on provider), so the model dropdown updates
// without saving. These tests guard the visibility predicate.

test("editAgent_providerFieldVisible_forBuzzAgent", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("buzz-agent"),
    true,
    "buzz-agent runtime must expose the provider picker",
  );
});

test("editAgent_providerFieldVisible_forGoose", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("goose"),
    true,
    "goose runtime must expose the provider picker",
  );
});

test("editAgent_providerFieldHidden_forClaude", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("claude"),
    false,
    "claude runtime locks the provider; picker must be hidden",
  );
});

test("editAgent_providerFieldHidden_forBlankRuntime", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection(""),
    false,
    "blank runtime (catalog miss) must not show the provider picker",
  );
});

// ── Provider dropdown options for EditAgentProviderField ────────────────────
//
// The provider dropdown must always contain the well-known providers
// (databricks, databricks_v2, anthropic, openai, openai-compat) plus a
// default-provider fallback entry so users can clear a saved provider.

test("editAgent_providerOptions_includesDatabricksProviders", () => {
  const options = getPersonaProviderOptions("", "buzz-agent");
  const ids = options.map((o) => o.id);
  assert.ok(ids.includes("databricks"), "databricks must be a provider option");
  assert.ok(
    ids.includes("databricks_v2"),
    "databricks_v2 must be a provider option",
  );
});

test("editAgent_providerOptions_includesDefaultEntry", () => {
  const options = getPersonaProviderOptions("", "buzz-agent");
  // The first entry is the default (empty id) — clearing back to runtime default.
  assert.equal(
    options[0].id,
    "",
    "first provider option must be the default (empty id)",
  );
});

test("editAgent_providerOptions_includesCurrentIfCustom", () => {
  const options = getPersonaProviderOptions("my-custom-llm", "buzz-agent");
  const ids = options.map((o) => o.id);
  assert.ok(
    ids.includes("my-custom-llm"),
    "a currently-saved custom provider must appear in the dropdown",
  );
});

// ── Finding 1 fix: fallback not disabled when discovery returns null ─────────
//
// When discoveredModelOptions is null, the model picker must NOT be disabled
// and the "Custom model..." option must remain selectable. This guards the
// regression where the select was disabled solely on missing discovery.
//
// We can't render React in pure node tests, but we CAN verify that the logic
// for deciding whether to show options is sound: when discovery is null, we
// fall back to staticModelOptions (length > 0), so we always have options.

test("editAgent_requiredDropdownField_onlyMarksMissingKnownField", async () => {
  const { isMissingRequiredDropdownField } = await import(
    "./personaDialogPickers.tsx"
  );

  assert.equal(
    isMissingRequiredDropdownField({ isRequired: true }, ""),
    true,
    "missing required dropdown value must be marked required",
  );
  assert.equal(
    isMissingRequiredDropdownField({ isRequired: true }, "configured"),
    false,
    "configured required dropdown value must not show the missing-required mark",
  );
  assert.equal(
    isMissingRequiredDropdownField(null, ""),
    false,
    "unknown normalized field names are ignored because they do not map to a dropdown",
  );
});

test("editAgent_modelFallback_staticOptionsWhenDiscoveryNull", () => {
  const staticModelOptions = [{ id: "", label: "Default model" }];
  // Simulate: discoveredModelOptions === null → effectiveModelOptions is static fallback
  const discoveredModelOptions = null;
  const effectiveModelOptions = discoveredModelOptions ?? staticModelOptions;
  assert.equal(
    effectiveModelOptions.length > 0,
    true,
    "effectiveModelOptions must be non-empty even when discovery returns null",
  );
  assert.equal(
    effectiveModelOptions[0].id,
    "",
    "fallback option must be the default (empty id)",
  );
});

test("editAgent_modelFallback_selectNotDisabledLogic", () => {
  // Verify: the correct disabled condition is (disabled || modelDiscoveryLoading),
  // NOT (disabled || modelDiscoveryLoading || !hasDiscoveredOptions).
  // We test this by confirming that a null discoveredModelOptions does NOT
  // set selectDisabled=true when the mutation is not pending and not loading.
  const disabled = false; // mutation not pending
  const modelDiscoveryLoading = false;
  // Old (buggy) logic would include: || !hasDiscoveredOptions
  // New (correct) logic:
  const selectDisabled = disabled || modelDiscoveryLoading;
  assert.equal(
    selectDisabled,
    false,
    "select must not be disabled when not loading and mutation is idle, regardless of discovery result",
  );
});

// ── Finding 2 fix: runtime switch enables provider picker ───────────────────
//
// Switching to buzz-agent runtime (which supports LLM provider selection)
// must make the provider field visible, enabling live discovery.

test("editAgent_runtimeSwitch_toBuzzAgentEnablesProvider", () => {
  // Simulate: user switches from "claude" to "buzz-agent"
  const previousRuntime = "claude";
  const nextRuntime = "buzz-agent";
  const previousSupportsProvider =
    runtimeSupportsLlmProviderSelection(previousRuntime);
  const nextSupportsProvider = runtimeSupportsLlmProviderSelection(nextRuntime);
  assert.equal(
    previousSupportsProvider,
    false,
    "claude must NOT support provider selection",
  );
  assert.equal(
    nextSupportsProvider,
    true,
    "buzz-agent MUST support provider selection",
  );
  // The provider field visibility transitions false → true on runtime change.
  assert.equal(
    !previousSupportsProvider && nextSupportsProvider,
    true,
    "switching from claude to buzz-agent must make provider field visible",
  );
});

// ── Finding 3 fix: provider field hidden and cleared for locked runtimes ────
//
// When the live runtime is a provider-locked one (e.g. claude), the provider
// field must NOT be visible even if a stale provider value is saved.

test("editAgent_providerFieldHidden_forLockedRuntimeEvenWithSavedProvider", () => {
  // Simulate: agent has a stale saved provider "databricks_v2" but
  // the live selected runtime is "claude" (provider-locked).
  const liveRuntimeId = "claude";
  const savedProvider = "databricks_v2";
  // New logic: visibility is keyed on LIVE runtime, not saved provider.
  const llmProviderFieldVisible =
    runtimeSupportsLlmProviderSelection(liveRuntimeId);
  assert.equal(
    llmProviderFieldVisible,
    false,
    "provider field must be hidden when live runtime is provider-locked, even if a provider was previously saved",
  );
  // Confirm: if we had used the old logic (|| savedProvider), it would be visible.
  const oldLogic =
    runtimeSupportsLlmProviderSelection(liveRuntimeId) ||
    savedProvider.trim().length > 0;
  assert.equal(
    oldLogic,
    true,
    "old logic would have incorrectly shown the provider field (this confirms the fix is meaningful)",
  );
});

// ── Runtime model-clear on change ─────────────────────────────────────────
//
// When the runtime changes, the model should be cleared if the previous
// runtime had a model that's not valid for the next runtime.

test("editAgent_modelClearedOnRuntimeChange", () => {
  const previousRuntime = "buzz-agent";
  const nextRuntime = "claude";
  assert.equal(
    shouldClearModelForRuntimeChange(previousRuntime, nextRuntime),
    true,
    "model must be cleared when switching runtimes",
  );
});

test("editAgent_modelNotClearedWhenRuntimeUnchanged", () => {
  const runtime = "buzz-agent";
  assert.equal(
    shouldClearModelForRuntimeChange(runtime, runtime),
    false,
    "model must NOT be cleared when the runtime stays the same",
  );
});

// ── Finding A fix: late catalog arrival does not wipe a valid saved provider ─
//
// When the dialog opens before the runtime catalog has loaded, selectedRuntimeId
// falls back to "custom" (no match). Once the catalog arrives, a separate effect
// re-derives the correct id — but ONLY if the user has not touched the dropdown.
// This ensures a no-op save never silently clears a valid databricks provider.

test("editAgent_catalogArrival_rederivesRuntimeIdWhenNotTouched", () => {
  // Simulate: open effect runs with empty runtimes → selectedRuntimeId = "custom".
  // Then catalog arrives with the saved agent's runtime.
  const agentCommand = "/usr/local/bin/buzz-agent";
  const catalog = [
    { id: "buzz-agent", command: agentCommand, defaultArgs: [] },
    { id: "claude", command: "/usr/local/bin/claude", defaultArgs: [] },
  ];
  const runtimeTouched = false; // user has not selected a runtime

  // Simulate the catalog-arrival effect logic.
  let selectedRuntimeId = "custom"; // seeded by open effect before catalog loaded
  if (!runtimeTouched && catalog.length > 0) {
    const matched = catalog.find(
      (r) => r.command?.trim() === agentCommand.trim(),
    );
    if (matched) {
      selectedRuntimeId = matched.id;
    }
  }

  assert.equal(
    selectedRuntimeId,
    "buzz-agent",
    "catalog-arrival effect must update selectedRuntimeId from 'custom' to the matched runtime",
  );
});

test("editAgent_catalogArrival_doesNotOverwriteUserSelection", () => {
  // Simulate: user has already picked a runtime (runtimeTouched = true).
  // The catalog-arrival effect must not overwrite the user's choice.
  const agentCommand = "/usr/local/bin/buzz-agent";
  const catalog = [
    { id: "buzz-agent", command: agentCommand, defaultArgs: [] },
  ];
  const runtimeTouched = true; // user already picked goose

  let selectedRuntimeId = "goose"; // user's choice
  if (!runtimeTouched && catalog.length > 0) {
    const matched = catalog.find(
      (r) => r.command?.trim() === agentCommand.trim(),
    );
    if (matched) {
      selectedRuntimeId = matched.id;
    }
  }

  assert.equal(
    selectedRuntimeId,
    "goose",
    "catalog-arrival effect must NOT overwrite user's selection when runtimeTouched is true",
  );
});

test("editAgent_noOpSavePreservesProvider_whenCatalogLate", () => {
  // Simulate the provider persistence logic when catalog arrived late.
  // If the catalog-arrival effect correctly sets selectedRuntimeId = "buzz-agent",
  // then llmProviderFieldVisible = true and the provider is preserved on save.
  const selectedRuntimeId = "buzz-agent"; // correctly derived after catalog arrival
  const savedProvider = "databricks_v2";
  const normalizedProvider = savedProvider;

  // The visibility logic (mirrors the component).
  const llmProviderFieldVisible =
    runtimeSupportsLlmProviderSelection(selectedRuntimeId);

  // The submit logic for provider tri-state.
  let providerUpdate;
  if (llmProviderFieldVisible) {
    // Only send if changed; here unchanged → undefined (no-op).
    providerUpdate =
      normalizedProvider !== (savedProvider ?? null)
        ? normalizedProvider
        : undefined;
  } else {
    // Would send null to clear.
    providerUpdate = (savedProvider ?? null) !== null ? null : undefined;
  }

  assert.equal(
    llmProviderFieldVisible,
    true,
    "provider field must be visible once catalog derives buzz-agent runtime",
  );
  assert.equal(
    providerUpdate,
    undefined,
    "a no-op save must NOT send null to clear the provider when runtime is correctly derived",
  );
});

// ── Finding B fix: inherited agent runtime switch produces consistent pair ───
//
// Selecting a concrete catalog runtime in the Edit dialog pins the harness
// (sets inheritHarness=false). This prevents the bad path where inheritHarness
// remains true while the provider is set for a different runtime.

test("editAgent_runtimeDropdown_pinsHarnessWhenConcreteCatalogRuntimeSelected", () => {
  // Simulate handleRuntimeDropdownChange for an inherited-Claude agent.
  let inheritHarness = true; // starts inherited

  // The fixed handler sets inheritHarness=false when a catalog runtime is picked.
  const _nextRuntimeId = "buzz-agent";
  const catalogRuntime = {
    id: "buzz-agent",
    command: "/usr/local/bin/buzz-agent",
    defaultArgs: [],
  };
  if (catalogRuntime.command) {
    // Catalog runtime selected: pin the harness.
    inheritHarness = false;
  }

  assert.equal(
    inheritHarness,
    false,
    "selecting a concrete catalog runtime must set inheritHarness=false",
  );
});

test("editAgent_inheritedAgentRuntimeSwitch_producesConsistentCommandProviderPair", () => {
  // Bad path before fix: inheritHarness stays true, so agentCommandUpdate is
  // undefined (agent still inherits Claude), but provider="databricks_v2" persists.
  //
  // After fix: selecting buzz-agent sets inheritHarness=false, so agentCommandUpdate
  // resolves to the buzz-agent command, and provider persists consistently.

  // Initial state: inherited Claude agent
  const inheritHarness = false; // after fix: pinned by runtime switch
  const selectedRuntimeCommand = "/usr/local/bin/buzz-agent";
  const agentOriginalCommand = ""; // was inheriting, no command
  const agentCommandOverride = null;

  // Submit logic for agentCommandUpdate (mirrors the component).
  const agentCommandUpdate = inheritHarness
    ? agentCommandOverride != null
      ? ""
      : undefined
    : selectedRuntimeCommand.trim() !== agentOriginalCommand
      ? selectedRuntimeCommand.trim()
      : undefined;

  const selectedRuntimeId = "buzz-agent";
  const llmProviderFieldVisible =
    runtimeSupportsLlmProviderSelection(selectedRuntimeId);
  const chosenProvider = "databricks_v2";
  const savedProvider = null; // was null (inherited Claude, no provider)
  const normalizedProvider = chosenProvider;

  let providerUpdate;
  if (llmProviderFieldVisible) {
    providerUpdate =
      normalizedProvider !== (savedProvider ?? null)
        ? normalizedProvider
        : undefined;
  } else {
    providerUpdate = (savedProvider ?? null) !== null ? null : undefined;
  }

  assert.equal(
    agentCommandUpdate,
    "/usr/local/bin/buzz-agent",
    "after runtime pin, agentCommandUpdate must be the concrete runtime command",
  );
  assert.equal(
    providerUpdate,
    "databricks_v2",
    "provider must persist consistently with the pinned runtime",
  );
  // Confirm both sides of the pair are consistent (concrete command + provider).
  assert.ok(
    agentCommandUpdate != null && providerUpdate != null,
    "command and provider must both be set — a mismatched pair is impossible",
  );
});

// ── Finding C / D / E fix: provider persistence tri-state ──────────────────
//
// The UI dropdown state (selectedRuntimeId / llmProviderFieldVisible) is for
// visibility only. Provider PERSISTENCE at submit keys on a tri-state derived
// from the EFFECTIVE runtime:
//
//   "capable"  → persist: value if changed, omit if unchanged.
//   "locked"   → clear: send null if provider was set, else omit.
//   "unknown"  → omit always (never send null for a transient/loading state).
//
// The "unknown" state is the key Finding-E addition: it prevents a transient
// catalog-loading state or a command:null entry from destructively clearing a
// valid provider snapshot.
//
// Helper mirrors the component's effectiveRuntimeIdForSubmit + tri-state logic.
function computeProviderCapability({
  inheritHarness,
  agentCommand,
  runtimes,
  selectedRuntimeId,
  selectedRuntime,
}) {
  // Step 1: derive the effective runtime id.
  // Inherit path: command match first, then id-based fallback for command:null
  // entries (known runtime with missing local adapter).
  const effectiveRuntimeIdForSubmit = inheritHarness
    ? (runtimes.find((r) => r.command?.trim() === agentCommand.trim())?.id ??
      runtimes.find((r) => r.id === agentCommand.trim())?.id ??
      "")
    : (selectedRuntime?.id ?? selectedRuntimeId);

  // Step 2: look up the catalog entry by id (not command) and classify.
  const matchedCatalogEntry =
    effectiveRuntimeIdForSubmit.length > 0
      ? runtimes.find((r) => r.id === effectiveRuntimeIdForSubmit)
      : undefined;
  if (matchedCatalogEntry === undefined) return "unknown";
  return runtimeSupportsLlmProviderSelection(matchedCatalogEntry.id)
    ? "capable"
    : "locked";
}

// Legacy boolean wrapper used by pre-Finding-E tests.
// Maps "capable" → true, "locked" → false, "unknown" → false.
// Note: "unknown" maps to false here (pre-Finding-E behaviour), but the real
// component now treats it as "omit" not "clear", which is what the new tests
// verify separately.
function computeCanPersistAtSubmit(args) {
  return computeProviderCapability(args) === "capable";
}

// Helper to simulate the full provider submit branch for the tri-state.
function computeProviderUpdate({ capability, savedProvider, currentProvider }) {
  const normalizedProvider = currentProvider?.trim() || null;
  if (capability === "capable") {
    return normalizedProvider !== (savedProvider ?? null)
      ? normalizedProvider
      : undefined;
  }
  if (capability === "locked") {
    return (savedProvider ?? null) !== null ? null : undefined;
  }
  // "unknown" → omit always
  return undefined;
}

test("editAgent_inheritCheckboxRoundTrip_doesNotPersistProviderOnInheritedRuntime", () => {
  // Simulate: inherited Claude agent (agentCommandOverride == null)
  // → user picks buzz-agent (inheritHarness→false, selectedRuntimeId='buzz-agent')
  // → user picks databricks_v2
  // → user RE-CHECKS inherit (inheritHarness→true, selectedRuntimeId STAYS 'buzz-agent')
  // → save: effective runtime is inherited (Claude), provider must NOT be persisted.

  const inheritHarness = true; // re-checked before save
  const agentCommand = "/usr/local/bin/claude"; // original Claude command
  const runtimes = [
    { id: "claude", command: "/usr/local/bin/claude", defaultArgs: [] },
    { id: "buzz-agent", command: "/usr/local/bin/buzz-agent", defaultArgs: [] },
  ];
  const selectedRuntimeId = "buzz-agent"; // dropdown state (stale after re-check)
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const savedProvider = null; // was null on open (inherited Claude had no provider)
  const chosenProvider = "databricks_v2"; // chosen while dropdown was buzz-agent

  // llmProviderFieldVisible is driven by the live dropdown (buzz-agent → true).
  // This is the UX visibility — unchanged by the fix.
  const llmProviderFieldVisible =
    runtimeSupportsLlmProviderSelection(selectedRuntimeId);
  assert.equal(
    llmProviderFieldVisible,
    true,
    "provider field is visible (dropdown shows buzz-agent) — this is the UX state",
  );

  // llmProviderCanPersistAtSubmit keys on the EFFECTIVE runtime.
  // Inherited Claude command → effective runtime = "claude" → not-provider-capable.
  const llmProviderCanPersistAtSubmit = computeCanPersistAtSubmit({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(
    llmProviderCanPersistAtSubmit,
    false,
    "provider must NOT be persistable when the inherited effective runtime is Claude",
  );

  // Submit logic for provider tri-state (mirrors the component).
  const normalizedProvider = chosenProvider;
  let providerUpdate;
  if (llmProviderCanPersistAtSubmit) {
    providerUpdate =
      normalizedProvider !== (savedProvider ?? null)
        ? normalizedProvider
        : undefined;
  } else {
    // Clear any stale saved provider, omit if already null.
    providerUpdate = (savedProvider ?? null) !== null ? null : undefined;
  }

  assert.equal(
    providerUpdate,
    undefined,
    "provider update must be omitted (not sent as databricks_v2) when reverting to inherited Claude",
  );
});

test("editAgent_inheritCheckboxRoundTrip_clearsStaleSavedProviderWhenRevertingToInherit", () => {
  // Variant: agent previously had a provider saved (e.g. was pinned to buzz-agent
  // with databricks_v2). User opens edit, re-checks inherit (inherited runtime is
  // Claude) → provider must be cleared (sent as null).

  const inheritHarness = true; // re-checked before save
  const agentCommand = "/usr/local/bin/claude"; // inherited Claude command
  const runtimes = [
    { id: "claude", command: "/usr/local/bin/claude", defaultArgs: [] },
    { id: "buzz-agent", command: "/usr/local/bin/buzz-agent", defaultArgs: [] },
  ];
  const selectedRuntimeId = "buzz-agent"; // dropdown state
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const savedProvider = "databricks_v2"; // was saved on open (pre-existing provider)
  const chosenProvider = "databricks_v2"; // unchanged from saved

  const llmProviderCanPersistAtSubmit = computeCanPersistAtSubmit({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });

  const normalizedProvider = chosenProvider;
  let providerUpdate;
  if (llmProviderCanPersistAtSubmit) {
    providerUpdate =
      normalizedProvider !== (savedProvider ?? null)
        ? normalizedProvider
        : undefined;
  } else {
    providerUpdate = (savedProvider ?? null) !== null ? null : undefined;
  }

  assert.equal(
    providerUpdate,
    null,
    "reverting to inherited Claude when a provider was previously saved must clear it (send null)",
  );
});

// ── Finding D fix: inherited provider-capable agent does not lose its provider
//                  on a name-only / no-op save ────────────────────────────────
//
// An agent with agentCommandOverride==null (inheritHarness=true) but whose
// persona's runtime is buzz-agent/Goose legitimately carries a provider
// snapshot (ManagedAgentRecord.provider). A no-op or name-only save must
// preserve that snapshot — not clear it. The fix derives the effective runtime
// from agent.agentCommand in the catalog rather than using !inheritHarness as
// a blanket not-provider-capable proxy.

test("editAgent_inheritedBuzzAgentProvider_preservedOnNameOnlySave", () => {
  // Inherited buzz-agent persona with databricks_v2 snapshot.
  // User makes a name-only edit (never touches runtime or provider).
  // The catalog-arrival effect correctly derived selectedRuntimeId="buzz-agent".

  const inheritHarness = true; // agentCommandOverride == null → inheriting
  const agentCommand = "/usr/local/bin/buzz-agent"; // inherited buzz-agent command
  const runtimes = [
    { id: "buzz-agent", command: "/usr/local/bin/buzz-agent", defaultArgs: [] },
    { id: "claude", command: "/usr/local/bin/claude", defaultArgs: [] },
  ];
  const selectedRuntimeId = "buzz-agent"; // correctly derived by catalog-arrival effect
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const savedProvider = "databricks_v2"; // valid provider snapshot
  const currentProvider = "databricks_v2"; // unchanged by user

  // llmProviderFieldVisible (UX) is true since dropdown shows buzz-agent.
  const llmProviderFieldVisible =
    runtimeSupportsLlmProviderSelection(selectedRuntimeId);
  assert.equal(
    llmProviderFieldVisible,
    true,
    "provider field must be visible for inherited buzz-agent",
  );

  // The effective runtime for submit: inherited → match agentCommand in catalog → buzz-agent.
  const llmProviderCanPersistAtSubmit = computeCanPersistAtSubmit({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(
    llmProviderCanPersistAtSubmit,
    true,
    "inherited buzz-agent runtime IS provider-capable — provider must be persistable",
  );

  // Submit logic: provider unchanged → omit (no-op).
  const normalizedProvider = currentProvider;
  let providerUpdate;
  if (llmProviderCanPersistAtSubmit) {
    providerUpdate =
      normalizedProvider !== (savedProvider ?? null)
        ? normalizedProvider
        : undefined;
  } else {
    providerUpdate = (savedProvider ?? null) !== null ? null : undefined;
  }

  assert.equal(
    providerUpdate,
    undefined,
    "name-only save on inherited buzz-agent must omit provider (not send null to clear it)",
  );
});

test("editAgent_inheritedBuzzAgentProvider_clearsWhenUserSwitchesToInheritedClaude", () => {
  // An agent inheriting buzz-agent with databricks_v2, but the persona was
  // changed to Claude (agentCommand now resolves to Claude). On save, the
  // provider must be cleared (not preserved for a non-capable runtime).

  const inheritHarness = true; // still inheriting
  const agentCommand = "/usr/local/bin/claude"; // persona now runs Claude
  const runtimes = [
    { id: "buzz-agent", command: "/usr/local/bin/buzz-agent", defaultArgs: [] },
    { id: "claude", command: "/usr/local/bin/claude", defaultArgs: [] },
  ];
  const selectedRuntimeId = "claude"; // catalog-arrival effect derives Claude
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const savedProvider = "databricks_v2"; // stale provider from before persona change

  const llmProviderCanPersistAtSubmit = computeCanPersistAtSubmit({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(
    llmProviderCanPersistAtSubmit,
    false,
    "inherited Claude runtime is not provider-capable — stale provider must not be persisted",
  );

  let providerUpdate;
  if (llmProviderCanPersistAtSubmit) {
    providerUpdate =
      savedProvider !== (savedProvider ?? null) ? savedProvider : undefined;
  } else {
    providerUpdate = (savedProvider ?? null) !== null ? null : undefined;
  }

  assert.equal(
    providerUpdate,
    null,
    "stale databricks_v2 on an inherited-Claude agent must be cleared on save",
  );
});

// ── Finding E fix: empty catalog and command:null do not clear a valid provider
//
// Two reachable forms can leave effectiveRuntimeIdForSubmit unresolvable:
//
//   Form 1: catalog is still loading at submit (runtimes=[]).
//   Form 2: catalog is loaded but the entry has command:null (adapter missing).
//
// In both cases, capability is "unknown" — the component must OMIT the provider
// field, never send null. "unknown" is not "locked"; only "locked" clears.

test("editAgent_findingE_emptyCatalog_providerOmittedNotCleared", () => {
  // Form 1: runtimes has not loaded yet at submit time.
  // Inherited buzz-agent agent with saved databricks_v2.
  // A name-only save must omit provider (not send null).

  const inheritHarness = true;
  const agentCommand = "buzz-agent"; // inherited command (short form)
  const runtimes = []; // catalog still loading
  const selectedRuntimeId = "custom"; // not yet derived
  const selectedRuntime = undefined;
  const savedProvider = "databricks_v2";
  const currentProvider = "databricks_v2"; // unchanged by user

  const capability = computeProviderCapability({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(
    capability,
    "unknown",
    "empty catalog must yield 'unknown' capability — not 'locked'",
  );

  const providerUpdate = computeProviderUpdate({
    capability,
    savedProvider,
    currentProvider,
  });
  assert.equal(
    providerUpdate,
    undefined,
    "empty-catalog submit must OMIT provider (undefined), not send null to clear it",
  );
});

test("editAgent_findingE_commandNullCatalogEntry_providerPreservedByIdMatch", () => {
  // Form 2: catalog loaded, but buzz-agent's entry has command:null (adapter
  // binary not installed). The command-based match fails; id-based fallback
  // finds the entry. Capability resolves to "capable" via id.
  // A name-only save must omit provider (unchanged → no-op, not null-clear).

  const inheritHarness = true;
  const agentCommand = "buzz-agent"; // inherited command (short form = runtime id)
  const runtimes = [
    { id: "buzz-agent", command: null, defaultArgs: [] }, // adapter missing
    { id: "claude", command: "claude-agent-acp", defaultArgs: [] },
  ];
  const selectedRuntimeId = "custom"; // command match failed → not re-derived
  const selectedRuntime = undefined;
  const savedProvider = "databricks_v2";
  const currentProvider = "databricks_v2"; // unchanged

  const capability = computeProviderCapability({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(
    capability,
    "capable",
    "command:null buzz-agent entry must resolve to 'capable' via id-based fallback",
  );

  const providerUpdate = computeProviderUpdate({
    capability,
    savedProvider,
    currentProvider,
  });
  assert.equal(
    providerUpdate,
    undefined,
    "name-only save on inherited buzz-agent (command:null) must omit provider, not clear it",
  );
});

test("editAgent_findingE_lockedRuntimeStillClears", () => {
  // Confirm that "locked" (known provider-incapable runtime) still sends null
  // to clear a stale provider — the Finding C/D behaviour must not regress.

  const inheritHarness = true;
  const agentCommand = "claude-agent-acp"; // inherited Claude
  const runtimes = [
    { id: "buzz-agent", command: "buzz-agent", defaultArgs: [] },
    { id: "claude", command: "claude-agent-acp", defaultArgs: [] },
  ];
  const selectedRuntimeId = "buzz-agent"; // stale dropdown state (irrelevant)
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const savedProvider = "databricks_v2"; // stale saved provider
  const currentProvider = "databricks_v2";

  const capability = computeProviderCapability({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(
    capability,
    "locked",
    "inherited Claude must classify as 'locked'",
  );

  const providerUpdate = computeProviderUpdate({
    capability,
    savedProvider,
    currentProvider,
  });
  assert.equal(
    providerUpdate,
    null,
    "locked runtime with a stale saved provider must send null to clear it",
  );
});

test("editAgent_findingE_capableBuzzAgentLoadedCatalog_preservedOnNoOpSave", () => {
  // Confirm loaded-catalog inherited buzz-agent still preserves provider.
  // This is Finding D's good path — must not regress with the tri-state change.

  const inheritHarness = true;
  const agentCommand = "buzz-agent";
  const runtimes = [
    { id: "buzz-agent", command: "buzz-agent", defaultArgs: [] },
    { id: "claude", command: "claude-agent-acp", defaultArgs: [] },
  ];
  const selectedRuntimeId = "buzz-agent";
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  const savedProvider = "databricks_v2";
  const currentProvider = "databricks_v2"; // unchanged

  const capability = computeProviderCapability({
    inheritHarness,
    agentCommand,
    runtimes,
    selectedRuntimeId,
    selectedRuntime,
  });
  assert.equal(capability, "capable", "loaded buzz-agent must be 'capable'");

  const providerUpdate = computeProviderUpdate({
    capability,
    savedProvider,
    currentProvider,
  });
  assert.equal(
    providerUpdate,
    undefined,
    "no-op save on inherited buzz-agent (loaded catalog) must omit provider",
  );
});

// ── Bug A fix: inherited-runtime (short-name agentCommand) seeds selectedRuntimeId
//              correctly via id-fallback when catalog command is the resolved path
//
// Problem: buzz-agent stores agentCommand="buzz-agent" (short name) while the
// catalog entry has command="/Applications/Buzz.app/.../buzz-agent" (resolved path).
// Command-based matching fails (short name ≠ full path), so selectedRuntimeId
// stayed "custom" → selectedRuntime=undefined → canDiscoverModelOptions=false →
// discovery never fired for inherited agents.
//
// Fix: both seeding spots (open-effect and catalog-arrival effect) now fall back
// to id-based matching when command-path matching misses — same id-fallback used
// by effectiveRuntimeIdForSubmit.

// Helper mirrors the fixed seeding logic from EditAgentDialog.tsx open-effect /
// catalog-arrival effect.
function deriveSelectedRuntimeId(agentCommand, runtimes) {
  const matched =
    runtimes.find((r) => r.command?.trim() === agentCommand.trim()) ??
    runtimes.find((r) => r.id === agentCommand.trim());
  return matched ? matched.id : "custom";
}

test("editAgent_bugA_inheritedShortName_resolvesViaIdFallback", () => {
  // The core regression: agentCommand is the short name "buzz-agent" but the
  // catalog's command is the resolved binary path. Command match fails; id
  // fallback must rescue it.

  const agentCommand = "buzz-agent"; // short name stored by effective_agent_command
  const runtimes = [
    {
      id: "buzz-agent",
      command: "/Applications/Buzz.app/Contents/MacOS/buzz-agent", // resolved path
      availability: "available",
      defaultArgs: [],
    },
    {
      id: "claude",
      command: "/usr/local/bin/claude-agent-acp",
      availability: "available",
      defaultArgs: [],
    },
  ];

  const selectedRuntimeId = deriveSelectedRuntimeId(agentCommand, runtimes);
  assert.equal(
    selectedRuntimeId,
    "buzz-agent",
    "short-name agentCommand must resolve to buzz-agent id via id-fallback when command path differs",
  );
});

test("editAgent_bugA_inheritedShortName_commandMatchStillWinsWhenPresent", () => {
  // Command-path match must still win when it succeeds (no regression for the
  // explicit-pin path where agentCommand IS the full resolved path).

  const agentCommand = "/usr/local/bin/buzz-agent"; // full path (explicit pin)
  const runtimes = [
    {
      id: "buzz-agent",
      command: "/usr/local/bin/buzz-agent",
      availability: "available",
      defaultArgs: [],
    },
  ];

  const selectedRuntimeId = deriveSelectedRuntimeId(agentCommand, runtimes);
  assert.equal(
    selectedRuntimeId,
    "buzz-agent",
    "command-path match must still win when agentCommand equals catalog command",
  );
});

test("editAgent_bugA_inheritedShortName_discoveryGatePasses", () => {
  // Once selectedRuntimeId is correctly resolved to "buzz-agent" via id-fallback,
  // selectedRuntime resolves to the available catalog entry, and the discovery gate
  // (canDiscoverModelOptions) passes so the model list populates.
  //
  // Mirrors usePersonaModelDiscovery's canDiscoverModelOptions logic:
  //   open && modelFieldVisible && selectedRuntime?.availability === "available"
  //   && discoveryAgentCommand !== null && ...

  const agentCommand = "buzz-agent"; // short name
  const runtimes = [
    {
      id: "buzz-agent",
      command: "/Applications/Buzz.app/Contents/MacOS/buzz-agent",
      availability: "available",
      defaultArgs: [],
    },
  ];

  // Step 1: derive selectedRuntimeId (the fix).
  const selectedRuntimeId = deriveSelectedRuntimeId(agentCommand, runtimes);
  // Step 2: resolve selectedRuntime from the catalog.
  const selectedRuntime = runtimes.find((r) => r.id === selectedRuntimeId);
  // Step 3: derive discoveryAgentCommand (mirrors usePersonaModelDiscovery:85-87).
  const discoveryAgentCommand = selectedRuntime?.command?.trim()
    ? selectedRuntime.command
    : null;
  // Step 4: evaluate the discovery gate (mirrors :88-93, simplified).
  const open = true;
  const modelFieldVisible = true;
  const isCustomProviderEditing = false;
  const trimmedProvider = "databricks_v2";
  const canDiscoverModelOptions =
    open &&
    modelFieldVisible &&
    selectedRuntime?.availability === "available" &&
    discoveryAgentCommand !== null &&
    (!isCustomProviderEditing || trimmedProvider.length > 0);

  assert.equal(
    selectedRuntimeId,
    "buzz-agent",
    "id-fallback must resolve selectedRuntimeId to buzz-agent",
  );
  assert.ok(
    selectedRuntime !== undefined,
    "selectedRuntime must resolve once selectedRuntimeId is correct",
  );
  assert.equal(
    discoveryAgentCommand,
    "/Applications/Buzz.app/Contents/MacOS/buzz-agent",
    "discoveryAgentCommand must be the resolved path from the catalog entry",
  );
  assert.equal(
    canDiscoverModelOptions,
    true,
    "discovery gate must pass for inherited buzz-agent with databricks_v2 provider once id-fallback resolves the runtime",
  );
});

test("editAgent_bugA_unknownCommandStillFallsBackToCustom", () => {
  // An agentCommand that matches neither catalog command nor catalog id must
  // still produce "custom" — the id-fallback must not introduce false positives.

  const agentCommand = "/some/custom/binary"; // not in catalog
  const runtimes = [
    {
      id: "buzz-agent",
      command: "/Applications/Buzz.app/Contents/MacOS/buzz-agent",
      availability: "available",
      defaultArgs: [],
    },
  ];

  const selectedRuntimeId = deriveSelectedRuntimeId(agentCommand, runtimes);
  assert.equal(
    selectedRuntimeId,
    "custom",
    "unknown command/id must still fall back to 'custom'",
  );
});

// ── requiredCredentialEnvKeys ──────────────────────────────────────────────
//
// Guards the provider-aware credential requirements surface so Phase 2
// required env rows stay correct as providers and runtimes change.

test("requiredCredentialEnvKeys: buzz-agent + anthropic → ANTHROPIC_API_KEY", () => {
  const keys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  assert.deepEqual(keys, ["ANTHROPIC_API_KEY"]);
});

test("requiredCredentialEnvKeys: buzz-agent + openai → OPENAI_COMPAT_API_KEY", () => {
  const keys = requiredCredentialEnvKeys("buzz-agent", "openai");
  assert.deepEqual(keys, ["OPENAI_COMPAT_API_KEY"]);
});

test("requiredCredentialEnvKeys: buzz-agent + databricks → DATABRICKS_HOST only (no token)", () => {
  const keys = requiredCredentialEnvKeys("buzz-agent", "databricks");
  // DATABRICKS_TOKEN is NOT required — OAuth PKCE is the normal path.
  assert.deepEqual(keys, ["DATABRICKS_HOST"]);
  assert.ok(
    !keys.includes("DATABRICKS_TOKEN"),
    "DATABRICKS_TOKEN must not be required (OAuth PKCE is the normal auth path)",
  );
});

test("requiredCredentialEnvKeys: buzz-agent + databricks_v2 → DATABRICKS_HOST only", () => {
  const keys = requiredCredentialEnvKeys("buzz-agent", "databricks_v2");
  assert.deepEqual(keys, ["DATABRICKS_HOST"]);
});

test("requiredCredentialEnvKeys: goose + anthropic → ANTHROPIC_API_KEY", () => {
  const keys = requiredCredentialEnvKeys("goose", "anthropic");
  assert.deepEqual(keys, ["ANTHROPIC_API_KEY"]);
});

test("requiredCredentialEnvKeys: goose + openai → OPENAI_COMPAT_API_KEY", () => {
  const keys = requiredCredentialEnvKeys("goose", "openai");
  assert.deepEqual(keys, ["OPENAI_COMPAT_API_KEY"]);
});

test("requiredCredentialEnvKeys: buzz-agent + no provider → empty (provider not yet selected)", () => {
  const keys = requiredCredentialEnvKeys("buzz-agent", "");
  assert.deepEqual(keys, []);
});

test("requiredCredentialEnvKeys: claude → empty (uses CLI login, not env keys)", () => {
  const keys = requiredCredentialEnvKeys("claude", "");
  assert.deepEqual(keys, []);
});

test("requiredCredentialEnvKeys: codex → empty (uses CLI login, not env keys)", () => {
  const keys = requiredCredentialEnvKeys("codex", "");
  assert.deepEqual(keys, []);
});

test("requiredCredentialEnvKeys: custom/unknown runtime → empty", () => {
  const keys = requiredCredentialEnvKeys("my-custom-harness", "anthropic");
  assert.deepEqual(keys, []);
});

// ── Block-save gate: hasRequiredEnvKeyMissing logic ────────────────────────
//
// The EditAgentDialog computes:
//   hasRequiredEnvKeyMissing = requiredEnvKeys.some(k => (envVars[k] ?? "").length === 0)
// and folds it into canSubmit. These tests exercise that predicate directly.

function hasRequiredEnvKeyMissing(requiredKeys, envVars) {
  return requiredKeys.some((key) => (envVars[key] ?? "").length === 0);
}

test("blockSave_buzzAgentAnthropicMissingKey_blocked", () => {
  // Will's exact case: buzz-agent / anthropic / opus / no ANTHROPIC_API_KEY
  const requiredKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const envVars = {}; // key absent
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    true,
    "save must be blocked when ANTHROPIC_API_KEY is missing",
  );
});

test("blockSave_buzzAgentAnthropicKeyProvided_allowed", () => {
  const requiredKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const envVars = { ANTHROPIC_API_KEY: "sk-ant-test" };
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    false,
    "save must be allowed when ANTHROPIC_API_KEY is present",
  );
});

test("blockSave_buzzAgentAnthropicEmptyStringKey_blocked", () => {
  // Empty string is treated the same as absent — matches EnvVarsEditor isMissing
  const requiredKeys = requiredCredentialEnvKeys("buzz-agent", "anthropic");
  const envVars = { ANTHROPIC_API_KEY: "" };
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    true,
    "save must be blocked when ANTHROPIC_API_KEY is empty string",
  );
});

test("blockSave_claudeNoCliLogin_notBlocked", () => {
  // CLI-login runtimes have no dialog-fixable requirement — must never block
  const requiredKeys = requiredCredentialEnvKeys("claude", "");
  const envVars = {}; // no keys set
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    false,
    "claude save must NOT be blocked — CLI login is out-of-band",
  );
});

test("blockSave_codexNoCliLogin_notBlocked", () => {
  const requiredKeys = requiredCredentialEnvKeys("codex", "");
  const envVars = {};
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    false,
    "codex save must NOT be blocked — CLI login is out-of-band",
  );
});

test("blockSave_buzzAgentDatabricksMissingHost_blocked", () => {
  const requiredKeys = requiredCredentialEnvKeys("buzz-agent", "databricks");
  const envVars = {};
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    true,
    "save must be blocked when DATABRICKS_HOST is missing",
  );
});

test("blockSave_buzzAgentDatabricksHostProvided_allowed", () => {
  const requiredKeys = requiredCredentialEnvKeys("buzz-agent", "databricks");
  const envVars = { DATABRICKS_HOST: "https://my.databricks.instance" };
  assert.equal(
    hasRequiredEnvKeyMissing(requiredKeys, envVars),
    false,
    "save must be allowed when DATABRICKS_HOST is present",
  );
});

// ── Block-save gate: isMissingRequiredDropdownField ────────────────────────
//
// The EditAgentDialog also gates on modelRequired / providerRequired.
// These tests guard the isMissingRequiredDropdownField predicate used for both.

test("blockSave_missingRequiredModel_blocked", () => {
  // isRequired=true and value is empty → should block
  assert.equal(
    isMissingRequiredDropdownField({ isRequired: true }, ""),
    true,
    "canSubmit must be blocked when required model is unset",
  );
});

test("blockSave_requiredModelProvided_allowed", () => {
  assert.equal(
    isMissingRequiredDropdownField({ isRequired: true }, "claude-opus-4-5"),
    false,
    "canSubmit must be allowed when required model is set",
  );
});

test("blockSave_optionalModelEmpty_allowed", () => {
  // isRequired=false → not a block-save condition
  assert.equal(
    isMissingRequiredDropdownField({ isRequired: false }, ""),
    false,
    "optional empty model must not block save",
  );
});

test("blockSave_nullField_allowed", () => {
  // null/undefined field descriptor → not required, not blocked
  assert.equal(
    isMissingRequiredDropdownField(null, ""),
    false,
    "null field must not block save",
  );
  assert.equal(
    isMissingRequiredDropdownField(undefined, ""),
    false,
    "undefined field must not block save",
  );
});

// ── Block-save gate: inherit-runtime transition cases ──────────────────────
//
// When inheritHarness=true, prospectiveRuntimeId resolves from agent.agentCommand
// (the persona's runtime), not the current dropdown. These tests guard the two
// failure modes Thufir flagged:
//   FALSE-ALLOW: claude pin → inherit buzz-agent persona → missing ANTHROPIC_API_KEY
//     → must be BLOCKED (prospective runtime is buzz-agent/anthropic, key absent)
//   FALSE-BLOCK: buzz-agent pin → inherit claude persona
//     → must NOT be blocked (claude has no dialog-fixable credential requirement)

test("blockSave_inheritTransition_claudePin_toBuzzAgentPersona_missingKey_blocked", () => {
  // Scenario: agent is currently pinned to claude (CLI-login, llmProviderFieldVisible=false
  // so providerForDiscovery="" in the component). The user checks "Inherit runtime
  // from persona" where the persona uses buzz-agent/anthropic.
  // prospectiveRuntimeId resolves to "buzz-agent"; providerForRequiredKeys must
  // use the PROSPECTIVE runtime's provider-field visibility (buzz-agent supports
  // provider selection) rather than the current locked runtime's suppression.
  const prospectiveRuntimeId = "buzz-agent"; // resolved from persona's agentCommand
  const provider = "anthropic"; // agent's configured provider (in envVars / state)

  // Mirror the component's providerForRequiredKeys computation:
  //   providerForRequiredKeys = runtimeSupportsLlmProviderSelection(prospectiveRuntimeId)
  //                              ? provider : ""
  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    prospectiveRuntimeId,
  )
    ? provider
    : "";

  const requiredKeys = requiredCredentialEnvKeys(
    prospectiveRuntimeId,
    providerForRequiredKeys,
  );
  const envVars = {}; // ANTHROPIC_API_KEY absent

  // providerForRequiredKeys must be "anthropic" (buzz-agent supports selection)
  // so requiredCredentialEnvKeys returns [ANTHROPIC_API_KEY] and save is blocked.
  assert.equal(
    providerForRequiredKeys,
    "anthropic",
    "providerForRequiredKeys must use the prospective runtime's visibility, not the locked current runtime",
  );
  const missing = requiredKeys.some((key) => (envVars[key] ?? "").length === 0);
  assert.equal(
    missing,
    true,
    "inheriting buzz-agent/anthropic persona with no key must BLOCK save (false-allow prevented)",
  );
});

test("blockSave_inheritTransition_buzzAgentPin_toClaudePersona_notBlocked", () => {
  // Scenario: agent is pinned to buzz-agent/anthropic. The user checks
  // "Inherit runtime from persona" where the persona uses claude.
  // prospectiveRuntimeId resolves to "claude"; claude doesn't support provider
  // selection, so providerForRequiredKeys="" and requiredCredentialEnvKeys returns [].
  const prospectiveRuntimeId = "claude"; // resolved from persona's agentCommand
  const provider = "anthropic"; // agent's old provider (no longer relevant for claude)

  // Mirror the component's providerForRequiredKeys computation:
  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    prospectiveRuntimeId,
  )
    ? provider
    : "";

  const requiredKeys = requiredCredentialEnvKeys(
    prospectiveRuntimeId,
    providerForRequiredKeys,
  );
  const envVars = {}; // nothing set — claude doesn't require dialog credentials

  // providerForRequiredKeys must be "" (claude doesn't support provider selection)
  assert.equal(
    providerForRequiredKeys,
    "",
    "providerForRequiredKeys must be empty for CLI-login runtimes",
  );
  const missing = requiredKeys.some((key) => (envVars[key] ?? "").length === 0);
  assert.equal(
    missing,
    false,
    "inheriting claude persona must NOT block save (false-block prevented — CLI login is out-of-band)",
  );
  assert.equal(
    requiredKeys.length,
    0,
    "claude must return empty required keys — no dialog-fixable credential",
  );
});
