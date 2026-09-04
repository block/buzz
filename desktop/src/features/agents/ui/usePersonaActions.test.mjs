import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { toast } from "sonner";
import { resolveLatestPersonaToEdit } from "./usePersonaActions.ts";
import { runAgentSaveCoordinator } from "./agentSaveCoordinator.ts";

// JSDOM globals for the one mounted-hook test below. The pure resolver/seam
// tests need no DOM; installing these is inert for them.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    localStorage: dom.window.localStorage,
  });
});

after(() => dom.window.close());

// ── P1-2 seam: Agents-library edit route must rebind ctx to the latest persona ─
//
// Thufir's IMPORTANT finding: the drift guard lives in the coordinator, but on
// the Agents-library route `ctx.definition` was fed the open-time snapshot, so
// a same-ID refresh never reached the guard and the clobber stayed reachable.
// `resolveLatestPersonaToEdit` closes that seam by re-deriving the live entity
// by ID. These tests exercise the REAL rebind wired into the REAL coordinator:
// reverting the rebind (returning the stored snapshot) makes the newer
// `updatedAt` invisible, the guard sees equality, and the write proceeds —
// which fails the abort assertions below.

function makeDefinition(overrides = {}) {
  return {
    id: "def-1",
    displayName: "Alice",
    avatarUrl: "",
    systemPrompt: "Be helpful.",
    runtime: "goose",
    model: "gpt-4o",
    provider: null,
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

function makePersonaInput(overrides = {}) {
  return {
    id: "def-1",
    displayName: "Alice-edited",
    systemPrompt: "Be helpful.",
    avatarUrl: "",
    runtime: "goose",
    model: "gpt-4o",
    provider: undefined,
    namePool: [],
    envVars: {},
    ...overrides,
  };
}

function captureToasts() {
  const captured = [];
  const original = {
    success: toast.success,
    warning: toast.warning,
    error: toast.error,
  };
  for (const kind of ["success", "warning", "error"]) {
    toast[kind] = (message) => captured.push({ kind, message });
  }
  return { captured, restore: () => Object.assign(toast, original) };
}

test("test_resolve_latest_persona_rebinds_to_refreshed_entity_by_id", () => {
  const snapshot = makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" });
  const refreshed = makeDefinition({
    updatedAt: "2025-06-01T00:00:00Z",
    systemPrompt: "Concurrently revised.",
  });

  const resolved = resolveLatestPersonaToEdit(snapshot, [refreshed]);
  assert.equal(
    resolved.updatedAt,
    "2025-06-01T00:00:00Z",
    "must rebind to the live entity so ctx advances with the store",
  );
});

test("test_resolve_latest_persona_falls_back_to_snapshot_when_deleted", () => {
  const snapshot = makeDefinition();
  // Persona vanished from query data (deleted while editing).
  assert.equal(
    resolveLatestPersonaToEdit(snapshot, []),
    snapshot,
    "must fall back to the stored snapshot when the id is gone",
  );
  assert.equal(resolveLatestPersonaToEdit(null, [makeDefinition()]), null);
});

test("test_agents_route_concurrent_definition_edit_aborts_before_any_write", async () => {
  // The full seam: user opens Alice from the Agents library (snapshot at T0,
  // captured as the seed-time updatedAt). A concurrent writer revises the same
  // definition to T1; personasQuery refreshes. The dialog rebinds ctx via
  // resolveLatestPersonaToEdit, then the user submits a definition edit.
  const cap = captureToasts();
  try {
    const snapshot = makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" });
    const seededUpdatedAt = snapshot.updatedAt; // captured at seed time

    const refreshed = makeDefinition({ updatedAt: "2025-06-01T00:00:00Z" });
    const ctxDefinition = resolveLatestPersonaToEdit(snapshot, [refreshed]);

    let definitionWrites = 0;
    const result = await runAgentSaveCoordinator({
      ctx: { kind: "definition-only", definition: ctxDefinition },
      personaInput: makePersonaInput(),
      agentInput: null,
      policySets: [],
      expectedDefinitionUpdatedAt: seededUpdatedAt,
      updatePersona: async () => {
        definitionWrites++;
      },
      updatePersonaAndPublish: async () => {
        definitionWrites++;
        return { publicationStatus: "published" };
      },
      updateManagedAgent: async () => ({ agent: null, profileSyncError: null }),
      setAutoRestart: async () => {},
      setStartOnAppLaunch: async () => {},
      refetchStores: async () => ({ persona: refreshed, agent: null }),
      onDone: () => {},
    });

    assert.equal(
      result,
      false,
      "drift must abort the save (dialog stays open)",
    );
    assert.equal(
      definitionWrites,
      0,
      "ZERO definition writes when ctx rebinds to a newer revision than the seed",
    );
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "one concurrent-edit error toast");
    assert.match(
      errors[0].message,
      /changed while you were editing/i,
      "must surface concurrent-edit messaging",
    );
  } finally {
    cap.restore();
  }
});

test("test_agents_route_no_concurrent_edit_proceeds_to_write", async () => {
  // No concurrent revision: the refreshed entity matches the seed revision, so
  // the guard does not fire and the definition write proceeds.
  const cap = captureToasts();
  try {
    const snapshot = makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" });
    const ctxDefinition = resolveLatestPersonaToEdit(snapshot, [snapshot]);

    let definitionWrites = 0;
    const result = await runAgentSaveCoordinator({
      ctx: { kind: "definition-only", definition: ctxDefinition },
      personaInput: makePersonaInput({ displayName: "Alice-edited" }),
      agentInput: null,
      policySets: [],
      expectedDefinitionUpdatedAt: snapshot.updatedAt,
      updatePersona: async () => {
        definitionWrites++;
      },
      updatePersonaAndPublish: async () => {
        definitionWrites++;
        return { publicationStatus: "published" };
      },
      updateManagedAgent: async () => ({ agent: null, profileSyncError: null }),
      setAutoRestart: async () => {},
      setStartOnAppLaunch: async () => {},
      refetchStores: async () => ({
        persona: makeDefinition({ displayName: "Alice-edited" }),
        agent: null,
      }),
      onDone: () => {},
    });

    assert.equal(result, true, "no drift → save succeeds");
    assert.equal(
      definitionWrites,
      1,
      "definition write proceeds when no concurrent revision",
    );
  } finally {
    cap.restore();
  }
});

// ── Mounted-hook pin: the memo-to-return wiring is what advances ctx ──────────
//
// The seam tests above call resolveLatestPersonaToEdit() directly, so they pass
// even if the production `useMemo`/return-field wiring in usePersonaActions is
// reverted (the clobber path Thufir restored under both mutations). This test
// renders the REAL hook: it selects at T0, replaces the persona query data with
// a same-ID T1, and asserts the value the hook RETURNS as `personaToEdit`
// advances to T1 — then feeds that returned value into the real coordinator and
// keeps the drift-abort assertion. Reverting either the memo or its returned
// field makes `personaToEdit` stay pinned at the T0 snapshot, so both the
// advance assertion and the coordinator abort fail.

async function renderPersonaActions(seedPersona) {
  const { createElement } = await import("react");
  const { renderHook } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  );

  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Infinity },
    },
  });
  // Seed every query the hook reads so no Tauri command fires. With no
  // community in localStorage the catalog query is disabled and its live
  // subscription is a no-op, so relayClient is never touched.
  qc.setQueryData(["personas"], [seedPersona]);
  qc.setQueryData(["identity"], { pubkey: "a".repeat(64) });
  qc.setQueryData(["acp-runtimes"], []);

  const rendered = renderHook(
    () => {
      const { usePersonaActions } = hookModule;
      return usePersonaActions();
    },
    {
      wrapper: ({ children }) =>
        createElement(
          QueryClientProvider,
          { client: qc },
          createElement(CommunitiesProvider, null, children),
        ),
    },
  );
  return { qc, ...rendered };
}

let hookModule;
before(async () => {
  hookModule = await import("./usePersonaActions.ts");
});

test("test_hook_returns_rebound_persona_and_coordinator_aborts_on_same_id_refresh", async () => {
  const { act } = await import("@testing-library/react");
  const cap = captureToasts();
  try {
    const snapshot = makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" });
    const seededUpdatedAt = snapshot.updatedAt; // captured at open time

    const { qc, result, unmount } = await renderPersonaActions(snapshot);

    // User clicks the persona in the Agents library at T0.
    act(() => result.current.openEdit(snapshot));
    assert.equal(
      result.current.personaToEdit?.updatedAt,
      seededUpdatedAt,
      "precondition: hook returns the open-time snapshot",
    );

    // A concurrent writer revises the SAME definition; personasQuery refreshes.
    const refreshed = makeDefinition({ updatedAt: "2025-06-01T00:00:00Z" });
    await act(async () => {
      qc.setQueryData(["personas"], [refreshed]);
      await Promise.resolve();
    });

    // The value the hook RETURNS must advance to the live revision — this is
    // the memo-to-return wiring the direct-resolver seam tests cannot pin.
    assert.equal(
      result.current.personaToEdit?.updatedAt,
      "2025-06-01T00:00:00Z",
      "the hook's returned personaToEdit must rebind to the refreshed revision",
    );

    // Feed exactly that returned value into the coordinator as ctx.definition.
    let definitionWrites = 0;
    const saveResult = await runAgentSaveCoordinator({
      ctx: {
        kind: "definition-only",
        definition: result.current.personaToEdit,
      },
      personaInput: makePersonaInput(),
      agentInput: null,
      policySets: [],
      expectedDefinitionUpdatedAt: seededUpdatedAt,
      updatePersona: async () => {
        definitionWrites++;
      },
      updatePersonaAndPublish: async () => {
        definitionWrites++;
        return { publicationStatus: "published" };
      },
      updateManagedAgent: async () => ({ agent: null, profileSyncError: null }),
      setAutoRestart: async () => {},
      setStartOnAppLaunch: async () => {},
      refetchStores: async () => ({ persona: refreshed, agent: null }),
      onDone: () => {},
    });

    assert.equal(saveResult, false, "drift must abort the save");
    assert.equal(
      definitionWrites,
      0,
      "ZERO definition writes when the hook rebinds ctx past the seed revision",
    );
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "one concurrent-edit error toast");
    assert.match(errors[0].message, /changed while you were editing/i);

    unmount();
  } finally {
    cap.restore();
  }
});
