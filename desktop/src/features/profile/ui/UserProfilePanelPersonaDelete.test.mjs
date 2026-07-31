/**
 * Ordering regression tests for the profile panel's persona delete.
 *
 * The invariant, and the bug this pins: a provider-backed instance must never
 * be left for `delete_persona`'s cascade to deal with. The backend cascade
 * refuses to delete a provider-deployed instance, so a persona that still has
 * one cannot be deleted at all — the call fails and the user is stuck. The fix
 * is ordering: the frontend tears every instance down through
 * `delete_managed_agent` (which prompts and passes `forceRemoteDelete`) and
 * only then calls `delete_persona`, by which point the cascade has nothing
 * provider-backed left to refuse.
 *
 * `managedAgentControlActions.test.mjs` covers the cascade helper in isolation
 * — that it visits the right instances and stops on a cancel or a failure.
 * What it cannot see is the ordering: a caller that ignored `cancelled` and
 * deleted the persona anyway, or one that deleted the persona first, passes
 * every one of those tests. That seam is here.
 *
 * The real hook is mounted (react-dom/client + act) over the real
 * `deleteProfileManagedAgentsForPersona` cascade and the real
 * `deleteManagedAgentWithRules` rules, so only the two backend leaves
 * (`delete_managed_agent`, `delete_persona`) and the channel cleanup are
 * faked. Every call lands in one ordered log, which is what the assertions
 * read — the order is the property under test, not the counts.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim ─────────────────────────────────────────────────────────
// react-dom/client needs a container element and a document; node has neither.
// The harness renders null, so no real node operations are exercised. Mirrors
// the shim in features/agents/ui/addCustomHarness.test.mjs, plus a `confirm`
// the tests swap — the orphan-warning prompt is what decides a cancelled
// cascade, and `deleteManagedAgentWithRules` reads it off `window`.

class ElementShim {
  constructor() {
    this.children = [];
    this.childNodes = [];
    this.nodeType = 1;
    this.nodeName = "DIV";
    this.tagName = "DIV";
    this.namespaceURI = "http://www.w3.org/1999/xhtml";
  }
  get ownerDocument() {
    return globalThis.document;
  }
  addEventListener() {}
  removeEventListener() {}
  appendChild(child) {
    this.children.push(child);
    this.childNodes.push(child);
    return child;
  }
  removeChild(child) {
    this.children = this.children.filter((current) => current !== child);
    this.childNodes = this.childNodes.filter((current) => current !== child);
    return child;
  }
  insertBefore(child) {
    return this.appendChild(child);
  }
  contains(target) {
    return this === target;
  }
}

globalThis.document = {
  activeElement: null,
  addEventListener() {},
  createElement: () => new ElementShim(),
  get defaultView() {
    return globalThis.window;
  },
  nodeType: 9,
  removeEventListener() {},
};
Object.defineProperty(globalThis, "window", {
  configurable: true,
  value: {
    addEventListener() {},
    // Replaced per test. Defaulting to a throw keeps a test that forgets to
    // set it from silently passing on an unprompted path.
    confirm: () => {
      throw new Error("window.confirm called without a test-supplied answer");
    },
    document: globalThis.document,
    event: undefined,
    HTMLIFrameElement: ElementShim,
    removeEventListener() {},
  },
});
globalThis.HTMLElement = ElementShim;
globalThis.Node = ElementShim;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { deleteProfileManagedAgentsForPersona } from "./UserProfilePanelDeletion.ts";
import { useProfilePersonaDelete } from "./UserProfilePanelPersonaDelete.ts";

const PERSONA = { id: "persona-1", displayName: "Scout" };

function instance(pubkeyChar, overrides = {}) {
  return {
    pubkey: pubkeyChar.repeat(64),
    name: "Scout instance",
    personaId: PERSONA.id,
    relayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    model: "hf://demo/model.gguf",
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: new Date(0).toISOString(),
    updatedAt: new Date(0).toISOString(),
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: false,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

/** An instance actually deployed to a provider — what the cascade refuses. */
function providerInstance(pubkeyChar) {
  return instance(pubkeyChar, {
    backend: { type: "provider" },
    backendAgentId: `remote-${pubkeyChar}`,
  });
}

/**
 * Mount the real hook over the real cascade. `deleteManagedAgent` and
 * `deletePersona` are the only faked backend calls; both append to `calls`, so
 * the log records the true interleaving the production code produced.
 *
 * No channels and no relay agents means every provider-backed instance
 * resolves to no channel and hits the orphan-warning confirm, which is the
 * branch a persona delete actually takes: `useProfileAgentDeletion` passes
 * `skipRemoteDeleteConfirm` only for a single-instance delete, never for the
 * persona cascade.
 */
async function mountPersonaDelete({
  managedAgents,
  confirm = () => true,
  deleteManagedAgent = async () => {},
}) {
  const calls = [];
  const control = {};
  globalThis.window.confirm = (message) => {
    calls.push({ op: "confirm", message });
    return confirm(message);
  };

  function Harness() {
    control.hook = useProfilePersonaDelete({
      deleteManagedAgentsForPersona: (persona) =>
        deleteProfileManagedAgentsForPersona(persona, {
          channels: [],
          deleteManagedAgent: async (input) => {
            calls.push({
              op: "delete_managed_agent",
              pubkey: input.pubkey,
              forceRemoteDelete: input.forceRemoteDelete,
            });
            return deleteManagedAgent(input);
          },
          managedAgents,
          presenceLookup: null,
          relayAgents: [],
          removeAgentFromAllChannels: async (pubkey) => {
            calls.push({ op: "remove_from_channels", pubkey });
          },
        }),
      deletePersona: async (id) => {
        calls.push({ op: "delete_persona", id });
      },
      managedAgents,
      onClose: () => {
        calls.push({ op: "close_panel" });
      },
    });
    return null;
  }

  const container = new ElementShim();
  const root = createRoot(container);
  await act(async () => {
    root.render(React.createElement(Harness));
  });

  return {
    calls,
    confirmDelete: async (persona = PERSONA) => {
      await act(async () => {
        await control.hook.handleConfirmDeletePersona(persona);
      });
    },
    hook: () => control.hook,
    setPersonaToDelete: async (persona) => {
      await act(async () => {
        control.hook.setPersonaToDelete(persona);
      });
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

const ops = (calls) => calls.map((call) => call.op);
const indexOfOp = (calls, op) => calls.findIndex((call) => call.op === op);

// ── The ordering invariant ───────────────────────────────────────────────────

test("every provider-backed instance is deleted before delete_persona runs", async () => {
  // Deliberately mixed and provider-last: the cascade must finish ALL of them,
  // not just the ones it happens to reach before the first local instance.
  const harness = await mountPersonaDelete({
    managedAgents: [
      providerInstance("a"),
      instance("b"),
      providerInstance("c"),
    ],
  });
  await harness.confirmDelete();

  const personaDeleteAt = indexOfOp(harness.calls, "delete_persona");
  assert.notEqual(personaDeleteAt, -1, "the persona must actually be deleted");

  const providerDeletes = harness.calls
    .map((call, index) => ({ call, index }))
    .filter(({ call }) => call.op === "delete_managed_agent")
    .filter(({ call }) => call.forceRemoteDelete === true);

  assert.deepEqual(
    providerDeletes.map(({ call }) => call.pubkey),
    ["a".repeat(64), "c".repeat(64)],
    "both provider-deployed instances must be torn down by the instance path",
  );
  for (const { call, index } of providerDeletes) {
    assert.ok(
      index < personaDeleteAt,
      `provider instance ${call.pubkey.slice(0, 4)} was still alive when delete_persona ran — the backend cascade would have refused it`,
    );
  }

  await harness.unmount();
});

test("delete_persona is the last backend call, after every instance", async () => {
  const harness = await mountPersonaDelete({
    managedAgents: [providerInstance("a"), instance("b")],
  });
  await harness.confirmDelete();

  assert.deepEqual(ops(harness.calls), [
    // Provider instance: orphan warning, forced delete, channel cleanup.
    "confirm",
    "delete_managed_agent",
    "remove_from_channels",
    // Local instance: no prompt.
    "delete_managed_agent",
    "remove_from_channels",
    // Only now is the persona safe to delete.
    "delete_persona",
    "close_panel",
  ]);

  await harness.unmount();
});

test("a provider-backed instance is deleted with forceRemoteDelete", async () => {
  // Without the force flag the backend's own orphan guard rejects the delete,
  // the instance survives, and the persona delete that follows hits exactly
  // the cascade refusal this ordering exists to avoid.
  const harness = await mountPersonaDelete({
    managedAgents: [providerInstance("a"), instance("b")],
  });
  await harness.confirmDelete();

  const deletes = harness.calls.filter(
    (call) => call.op === "delete_managed_agent",
  );
  assert.equal(deletes[0].forceRemoteDelete, true, "provider instance forces");
  assert.equal(
    deletes[1].forceRemoteDelete,
    undefined,
    "a local instance must not force — it has no remote deployment to orphan",
  );

  await harness.unmount();
});

// ── Aborts must not fall through to the persona delete ───────────────────────

test("declining the provider orphan warning never deletes the persona", async () => {
  const harness = await mountPersonaDelete({
    managedAgents: [providerInstance("a"), instance("b")],
    confirm: () => false,
  });
  await harness.confirmDelete();

  assert.equal(
    indexOfOp(harness.calls, "delete_persona"),
    -1,
    "cancelling the cascade must abort the persona delete, not proceed without it",
  );
  assert.deepEqual(ops(harness.calls), ["confirm"]);

  await harness.unmount();
});

test("a failed instance delete never deletes the persona", async () => {
  const harness = await mountPersonaDelete({
    managedAgents: [providerInstance("a"), instance("b")],
    deleteManagedAgent: async () => {
      throw new Error("backend refused");
    },
  });
  // The hook reports failures through a toast rather than rejecting, so the
  // absence of delete_persona is the only observable proof it aborted.
  await harness.confirmDelete();

  assert.equal(
    indexOfOp(harness.calls, "delete_persona"),
    -1,
    "a throw mid-cascade must not fall through to the persona delete",
  );

  await harness.unmount();
});

test("a team-managed persona deletes nothing at all", async () => {
  const harness = await mountPersonaDelete({
    managedAgents: [providerInstance("a")],
  });
  await harness.confirmDelete({ ...PERSONA, sourceTeam: "team-1" });

  assert.deepEqual(
    harness.calls,
    [],
    "the team guard must run before the cascade, not after it destroys instances",
  );

  await harness.unmount();
});

// ── The counts the confirm dialog discloses ──────────────────────────────────

test("providerInstanceCount counts only instances deployed to a provider", async () => {
  const harness = await mountPersonaDelete({
    managedAgents: [
      providerInstance("a"),
      instance("b"),
      // Configured for a provider but never deployed: no remote deployment
      // exists, so warning about one would be false.
      instance("c", { backend: { type: "provider" }, backendAgentId: null }),
      instance("d", { personaId: "persona-2" }),
    ],
  });
  await harness.setPersonaToDelete(PERSONA);

  assert.equal(harness.hook().instanceCount, 3, "excludes other personas");
  assert.equal(harness.hook().providerInstanceCount, 1);

  await harness.unmount();
});

test("both counts are zero while no persona is pending deletion", async () => {
  const harness = await mountPersonaDelete({
    managedAgents: [providerInstance("a")],
  });

  assert.equal(harness.hook().instanceCount, 0);
  assert.equal(harness.hook().providerInstanceCount, 0);

  await harness.unmount();
});
