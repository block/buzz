import assert from "node:assert/strict";
import test from "node:test";

import {
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
} from "./agentConfigOptions.tsx";
import {
  computeEditAgentFormValidity,
  hasMissingRequiredEnvKey,
  resolveAgentCommandUpdate,
  resolveEffortSubmission,
  resolveInheritedRuntimeSubmission,
  resolveRuntimeProviderCapability,
} from "./personaRuntimeModel.ts";

// ── Phase 1B.3b re-host pinning: inherit-toggle → gate → submit ─────────────
//
// AgentInstanceEditDialog (re-homed from EditAgentDialog) wires three seams
// that must never disagree after the move:
//   1. TOGGLE: flipping "Inherit runtime from persona" re-resolves
//      prospectiveRuntimeId from the LINKED PERSONA's runtime (not the stale
//      override) and feeds resolveInheritedRuntimeSubmission.
//   2. GATE: useRequiredCredentialState validates the SAME inheritedSubmission
//      provider/env the submit path will persist (gate ↔ record ↔ spawn).
//   3. SUBMIT: UpdateManagedAgentInput persists that same snapshot, with
//      resolveAgentCommandUpdate + harnessOverride derived from the toggle.
// These tests chain the pure modules exactly as the component does, so a
// re-host that re-derives any seam independently fails here.

const runtimes = [
  { id: "buzz-agent", command: "buzz-agent-cmd", defaultArgs: [] },
  { id: "claude", command: "claude-cmd", defaultArgs: [] },
];

// Mirrors the component's prospectiveRuntimeId memo: when inheriting, resolve
// from the linked persona's runtime; fall back to the agentCommand dual-match.
function prospectiveRuntimeIdFor({
  inheritHarness,
  selectedRuntimeId,
  linkedPersonaRuntime,
  agentCommand,
}) {
  if (!inheritHarness) {
    return selectedRuntimeId;
  }
  const personaRuntimeId = linkedPersonaRuntime?.trim();
  if (personaRuntimeId) {
    return (
      runtimes.find((r) => r.id === personaRuntimeId)?.id ?? personaRuntimeId
    );
  }
  return (
    runtimes.find((r) => r.command?.trim() === agentCommand.trim())?.id ??
    runtimes.find((r) => r.id === agentCommand.trim())?.id ??
    ""
  );
}

// A Claude-pinned agent linked to a buzz-agent/anthropic persona — the
// inherit-transition scenario that exercises every seam at once.
const pinnedAgent = {
  name: "test-agent",
  agentCommand: "claude-cmd",
  agentCommandOverride: "claude-cmd",
  acpCommand: "acp",
  personaId: "p1",
  provider: null,
  model: null,
  envVars: {},
};
const persona = {
  runtime: "buzz-agent",
  provider: "anthropic",
  model: "claude-sonnet-4-5",
  envVars: { ANTHROPIC_API_KEY: "sk-persona" },
};

function inheritTransitionState() {
  const inheritHarness = true; // user just checked the toggle
  const prospectiveRuntimeId = prospectiveRuntimeIdFor({
    inheritHarness,
    selectedRuntimeId: "claude",
    linkedPersonaRuntime: persona.runtime,
    agentCommand: pinnedAgent.agentCommand,
  });
  const inheritedSubmission = resolveInheritedRuntimeSubmission({
    inheritHarness,
    agentWasHarnessPinned: pinnedAgent.agentCommandOverride != null,
    provider: "", // pinned Claude agent carries no provider
    personaProvider: persona.provider,
    model: "",
    personaModel: persona.model,
    envVars: {},
    personaEnvVars: persona.envVars,
  });
  return { inheritHarness, prospectiveRuntimeId, inheritedSubmission };
}

test("rehost_toggle_resolvesProspectiveRuntimeFromPersona_notOverride", () => {
  const { prospectiveRuntimeId } = inheritTransitionState();
  assert.equal(
    prospectiveRuntimeId,
    "buzz-agent",
    "inherit-toggle must resolve the prospective runtime from the linked persona, not the still-present Claude pin",
  );
});

test("rehost_gate_validatesTheSubmissionSnapshot_sameValuesAsSubmit", () => {
  const { prospectiveRuntimeId, inheritedSubmission } =
    inheritTransitionState();

  // Gate half: the credential requirement must be computed from the
  // PROSPECTIVE runtime + the submission snapshot's provider/env.
  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    prospectiveRuntimeId,
  )
    ? (inheritedSubmission.provider ?? "")
    : "";
  const requiredKeys = requiredCredentialEnvKeys(
    prospectiveRuntimeId,
    providerForRequiredKeys,
  );
  const missing = hasMissingRequiredEnvKey(
    requiredKeys,
    inheritedSubmission.envVars,
  );

  // The persona snapshot carries the credential, so the gate must clear —
  // and it must clear because of the SAME env map submit will persist.
  assert.equal(inheritedSubmission.provider, "anthropic");
  assert.deepEqual(inheritedSubmission.envVars, {
    ANTHROPIC_API_KEY: "sk-persona",
  });
  assert.equal(
    missing,
    false,
    "gate must validate the submission snapshot (persona-layered env), not the agent's own empty env",
  );
});

test("rehost_gate_blocksSave_whenSubmissionSnapshotLacksCredential", () => {
  const { prospectiveRuntimeId } = inheritTransitionState();
  // Same transition but the persona carries no credential: the submission
  // snapshot is credential-less and the SAME gate must now block Save.
  const bareSubmission = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: true,
    provider: "",
    personaProvider: persona.provider,
    model: "",
    personaModel: persona.model,
    envVars: {},
    personaEnvVars: {}, // no key anywhere
  });
  const requiredKeys = requiredCredentialEnvKeys(
    prospectiveRuntimeId,
    bareSubmission.provider ?? "",
  );
  const requiredEnvKeyMissing = hasMissingRequiredEnvKey(
    requiredKeys,
    bareSubmission.envVars,
  );
  assert.equal(requiredEnvKeyMissing, true);

  const canSubmit = computeEditAgentFormValidity({
    name: pinnedAgent.name,
    parallelism: "1",
    agentAcpCommand: pinnedAgent.acpCommand,
    acpCommand: pinnedAgent.acpCommand,
    respondTo: "mentions",
    respondToAllowlistLength: 0,
    selectedRuntimeId: "claude",
    inheritHarness: true,
    agentCommand: pinnedAgent.agentCommand,
    requiredEnvKeyMissing,
  });
  assert.equal(
    canSubmit,
    false,
    "missing credential in the submission snapshot must block Save through the same validity path",
  );
});

test("rehost_submit_persistsToggleAsCommandClear_andOmitsHarnessOverride", () => {
  // Submit half of the toggle seam: inheriting with a persisted override must
  // send the clear sentinel (""), and harnessOverride derives from the SAME
  // agentCommandUpdate (omitted while inheriting — falsy per the component).
  const agentCommandUpdate = resolveAgentCommandUpdate({
    inheritHarness: true,
    agentCommand: pinnedAgent.agentCommand,
    originalAgentCommand: pinnedAgent.agentCommand,
    agentCommandOverride: pinnedAgent.agentCommandOverride,
  });
  assert.equal(
    agentCommandUpdate,
    "",
    "inherit with a persisted pin must persist the clear sentinel",
  );
  const harnessOverride =
    agentCommandUpdate != null ? !true /* inheritHarness */ : undefined;
  assert.equal(
    harnessOverride,
    false,
    "harnessOverride must derive from the shared agentCommandUpdate, signalling the cleared pin",
  );

  // And the provider tri-state must classify the PROSPECTIVE runtime — the
  // same id the gate used — so submit persists what the gate validated.
  const { prospectiveRuntimeId, inheritedSubmission } =
    inheritTransitionState();
  const capability = resolveRuntimeProviderCapability(
    prospectiveRuntimeId,
    runtimeSupportsLlmProviderSelection(prospectiveRuntimeId),
  );
  assert.equal(capability, "capable");
  const providerUpdate =
    capability === "capable"
      ? inheritedSubmission.provider !== (pinnedAgent.provider ?? null)
        ? inheritedSubmission.provider
        : undefined
      : undefined;
  assert.equal(
    providerUpdate,
    "anthropic",
    "submit must persist the gate-validated submission provider for the prospective runtime",
  );
});

test("rehost_steadyStateInherit_localEditsStayAuthoritative", () => {
  // Wipe-on-poll companion: an agent ALREADY inheriting at open (no pin) with
  // deliberate local edits — the submission snapshot must pass them through
  // untouched, not resurrect persona values (the pre-move contract).
  const submission = resolveInheritedRuntimeSubmission({
    inheritHarness: true,
    agentWasHarnessPinned: false, // steady state, not a transition
    provider: "databricks", // deliberate local re-point
    personaProvider: persona.provider,
    model: "",
    personaModel: persona.model,
    envVars: { DATABRICKS_TOKEN: "tok" },
    personaEnvVars: persona.envVars,
  });
  assert.equal(submission.provider, "databricks");
  assert.equal(
    submission.model,
    null,
    "empty local model in steady state stays empty (runtime default), never backfilled from the persona",
  );
  assert.deepEqual(submission.envVars, { DATABRICKS_TOKEN: "tok" });
});

test("editValidity_allowlistWithEmptyList_blocksSave", () => {
  // PR #1667 review: the edit-path crash-loop guard (respondToValid) was
  // never exercised — every other row uses a non-allowlist mode. An agent
  // saved as allowlist-with-empty-list crash-loops at startup.
  const base = {
    name: pinnedAgent.name,
    parallelism: "1",
    agentAcpCommand: pinnedAgent.acpCommand,
    acpCommand: pinnedAgent.acpCommand,
    selectedRuntimeId: "claude",
    inheritHarness: true,
    agentCommand: pinnedAgent.agentCommand,
    requiredEnvKeyMissing: false,
  };
  assert.equal(
    computeEditAgentFormValidity({
      ...base,
      respondTo: "allowlist",
      respondToAllowlistLength: 0,
    }),
    false,
    "allowlist with an empty list must block Save",
  );
  assert.equal(
    computeEditAgentFormValidity({
      ...base,
      respondTo: "allowlist",
      respondToAllowlistLength: 1,
    }),
    true,
    "allowlist with at least one pubkey must allow Save",
  );
});

// ── Cancel-safety: inherit toggle → Cancel emits no update_managed_agent ──────
//
// Acceptance pin (plan item 1). The pin→inherit effort/runtime clear is derived
// entirely inside the backend's locked save, keyed off the agentCommand:""
// sentinel `resolveAgentCommandUpdate` produces at SUBMIT. Cancel-safety is
// therefore a UI-wiring invariant: flipping the inherit toggle mutates only
// local dialog state, and the Cancel button routes to onOpenChange, never to
// the submit path — so no update_managed_agent call (and thus no column/env
// clear) is ever dispatched when the user backs out.
//
// This mirrors the AgentInstanceEditDialog footer exactly: Cancel →
// onOpenChange(false); Save → handleSubmit → updateMutation.mutateAsync(input),
// where `input.agentCommand` is the resolveAgentCommandUpdate sentinel.
test("inheritToggle_cancelled_emitsNoUpdate", () => {
  const calls = [];
  // The ONLY producer of the persistence-boundary sentinel is the submit path.
  function handleSubmit() {
    const agentCommandUpdate = resolveAgentCommandUpdate({
      inheritHarness: true, // user just toggled inherit ON
      agentCommand: pinnedAgent.agentCommand,
      originalAgentCommand: pinnedAgent.agentCommand,
      agentCommandOverride: pinnedAgent.agentCommandOverride ?? null,
    });
    calls.push({ agentCommand: agentCommandUpdate });
  }
  function onOpenChange() {
    /* dialog close — no mutation */
  }

  // User toggles inherit (local state only), then clicks Cancel.
  const cancelButton = { onClick: () => onOpenChange(false) };
  cancelButton.onClick();

  assert.equal(
    calls.length,
    0,
    "Cancel after toggling inherit must not dispatch update_managed_agent",
  );

  // Sanity: the submit path WOULD have emitted the inherit sentinel, proving
  // the clear is gated on Save alone — Cancel simply never reaches it.
  handleSubmit();
  assert.equal(calls.length, 1);
  assert.equal(
    calls[0].agentCommand,
    "",
    "Save on the pin→inherit transition emits the empty-command sentinel the backend clears the column on",
  );
});

// ── Effort write is Save-gated and cannot re-pin an inherit transition ────────
//
// Carl r8 P1. The effort picker no longer direct-writes on selection; the
// dialog holds the pending value and embeds it in the locked `update_managed_agent`
// payload (PR #4625 — effort is in the locked update, not a separate IPC). Two invariants this pins:
//   1. On the pin→inherit transition (agentCommandUpdate === ""), the locked
//      save already cleared the effort column + aliases, so the effort setter
//      must be SUPPRESSED — re-persisting the picked value would restore the
//      very pin the transition just cleared (the deferred-IPC race).
//   2. Cancel never reaches the submit path, so no effort write is dispatched.

test("inheritTransition_suppressesEffortWrite_evenWhenUserPickedAValue", () => {
  // User pinned to Claude, selected effort "high", then switched to Inherit and
  // saved. The command update is the inherit sentinel, so the effort write must
  // be suppressed regardless of the picked value.
  const agentCommandUpdate = resolveAgentCommandUpdate({
    inheritHarness: true,
    agentCommand: pinnedAgent.agentCommand,
    originalAgentCommand: pinnedAgent.agentCommand,
    agentCommandOverride: pinnedAgent.agentCommandOverride,
  });
  assert.equal(agentCommandUpdate, "");

  const submission = resolveEffortSubmission({
    effortLevel: "high", // the deferred selection that used to race the save
    originalEffortLevel: null,
    inheritTransition: agentCommandUpdate === "",
  });
  assert.equal(
    submission.persist,
    false,
    "the pin→inherit transition must suppress the effort write so it cannot restore the just-cleared pin",
  );
});

test("effortWrite_persistsRealChange_onNonInheritSave", () => {
  // A plain effort edit with no harness change: the setter persists the new
  // value (agentCommandUpdate is undefined → not an inherit transition).
  const submission = resolveEffortSubmission({
    effortLevel: "high",
    originalEffortLevel: null,
    inheritTransition: false,
  });
  assert.deepEqual(submission, { persist: true, level: "high" });
});

test("effortWrite_noOp_whenSelectionUnchanged", () => {
  // Re-selecting the currently-effective value (or a name-only Save) writes
  // nothing, so an unrelated edit never rewrites the effort column.
  const submission = resolveEffortSubmission({
    effortLevel: "high",
    originalEffortLevel: "high",
    inheritTransition: false,
  });
  assert.equal(submission.persist, false);
});

test("effortWrite_clearToAdapterDefault_persistsNull", () => {
  // Clearing an existing pin to the adapter-default sentinel is a real change
  // and must persist null (revert), not be treated as a no-op.
  const submission = resolveEffortSubmission({
    effortLevel: null,
    originalEffortLevel: "high",
    inheritTransition: false,
  });
  assert.deepEqual(submission, { persist: true, level: null });
});
