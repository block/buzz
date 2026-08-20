/**
 * Ordering regression tests for the AgentsView persona delete
 * (`usePersonaActions.handleDelete`).
 *
 * Same invariant as the profile panel's
 * `UserProfilePanelPersonaDelete.test.mjs`, on the other surface that deletes
 * a persona: `delete_persona` must not be reached while any provider-deployed
 * instance is still alive. The backend cascade refuses to delete one, so the
 * persona delete fails outright and the user is stuck with a persona they
 * cannot remove.
 *
 * `handleDelete` takes its instance teardown as a required parameter, so a
 * caller cannot skip it without a type error — that much the compiler already
 * guards. What the compiler cannot see is the body: whether the teardown is
 * awaited *before* `delete_persona`, and whether a cancelled or failed
 * teardown actually aborts. Those three are what this file pins.
 *
 * The real hook is mounted (react-dom/client + act) over a real QueryClient
 * and the real CommunitiesProvider, with Tauri IPC intercepted at
 * `__TAURI_INTERNALS__.invoke` by command name — so `delete_persona` is
 * observed where the production code actually issues it, not at a stub in
 * between. The instance teardown is the injected fake, which is exactly the
 * seam under test.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim ─────────────────────────────────────────────────────────
// react-dom/client needs a container element and a document; node has neither.
// The harness renders null, so no real node operations are exercised. Mirrors
// the shim in addCustomHarness.test.mjs, plus the localStorage
// CommunitiesProvider reads on mount.

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

const localStorageShim = (() => {
  const store = new Map();
  return {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
    removeItem: (key) => store.delete(key),
  };
})();

Object.defineProperty(globalThis, "window", {
  configurable: true,
  value: {
    addEventListener() {},
    document: globalThis.document,
    event: undefined,
    HTMLIFrameElement: ElementShim,
    localStorage: localStorageShim,
    removeEventListener() {},
  },
});
globalThis.localStorage = localStorageShim;
globalThis.HTMLElement = ElementShim;
globalThis.Node = ElementShim;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

// ── Tauri IPC mock ───────────────────────────────────────────────────────────
// @tauri-apps/api/core calls window.__TAURI_INTERNALS__.invoke(cmd, args).
// Intercepting here catches `delete_persona` at the real call site rather than
// at a stubbed mutation, and lets the persona-delete command land in the same
// log as the injected teardown. `onInvoke` is reassigned per mount.
//
// It must hang off the window shim above, not just globalThis: the shim is a
// distinct object, so an internals stub installed only on globalThis is
// invisible to @tauri-apps/api and every command silently fails to dispatch.
const OWNER_PUBKEY = "1f".repeat(32);

/**
 * Per-command responses. The hook's queries run for real once IPC dispatches,
 * so each command answers in its true shape — `get_identity` in particular
 * returns an object, not an empty array. Returning a blanket `[]` here makes
 * `identityQuery.data.pubkey` undefined and crashes the mount inside a
 * `useMemo`, which looks like a product bug but is purely a stub artefact.
 * Anything unlisted resolves to undefined, which every consumer already
 * guards with `?.` or `?? []`.
 */
const IPC_RESPONSES = {
  get_identity: {
    pubkey: OWNER_PUBKEY,
    display_name: "Owner",
    lost: false,
    locked: false,
    reset_failed: false,
  },
  list_personas: [],
  list_managed_agents: [],
  list_acp_runtimes: [],
};

let onInvoke = async (cmd) => IPC_RESPONSES[cmd];
const tauriInternals = {
  invoke: (cmd, args) => onInvoke(cmd, args),
  transformCallback: (callback) => callback,
};
globalThis.__TAURI_INTERNALS__ = tauriInternals;
globalThis.window.__TAURI_INTERNALS__ = tauriInternals;

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { CommunitiesProvider } from "@/features/communities/useCommunities.tsx";
import { usePersonaActions } from "./usePersonaActions.ts";

const PERSONA = { id: "persona-1", displayName: "Scout" };

/**
 * Mount the real hook. `deleteInstances` stands in for
 * `handleDeleteInstancesForPersona`; every Tauri command and every teardown
 * boundary lands in one ordered log, which is what the assertions read.
 */
async function mountPersonaActions({ deleteInstances }) {
  const calls = [];
  const control = {};

  onInvoke = async (cmd) => {
    // The hook's queries fire reads on mount; logging only the destructive
    // command keeps the log to the sequence under test.
    if (cmd === "delete_persona") calls.push("delete_persona");
    return IPC_RESPONSES[cmd];
  };

  function Harness() {
    control.hook = usePersonaActions();
    return null;
  }

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const root = createRoot(new ElementShim());
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(Harness),
        ),
      ),
    );
  });

  return {
    calls,
    delete: async (persona = PERSONA) => {
      await act(async () => {
        await control.hook.handleDelete(persona, async (target) => {
          calls.push("teardown:start");
          const result = await deleteInstances(target);
          calls.push("teardown:end");
          return result;
        });
      });
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
    },
  };
}

// ── The ordering invariant ───────────────────────────────────────────────────

test("the persona is deleted only after instance teardown completes", async () => {
  const harness = await mountPersonaActions({
    deleteInstances: async () => ({ deletedCount: 2 }),
  });
  await harness.delete();

  assert.deepEqual(harness.calls, [
    "teardown:start",
    "teardown:end",
    "delete_persona",
  ]);

  await harness.unmount();
});

test("teardown is awaited, not merely started, before delete_persona", async () => {
  // A teardown that resolves on a later tick: if the persona delete were
  // issued without awaiting it, delete_persona would land between the two
  // teardown markers rather than after both.
  const harness = await mountPersonaActions({
    deleteInstances: async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
      return { deletedCount: 1 };
    },
  });
  await harness.delete();

  assert.deepEqual(harness.calls, [
    "teardown:start",
    "teardown:end",
    "delete_persona",
  ]);

  await harness.unmount();
});

// ── Aborts must not fall through to the persona delete ───────────────────────

test("a cancelled teardown never deletes the persona", async () => {
  const harness = await mountPersonaActions({
    deleteInstances: async () => ({ cancelled: true, deletedCount: 0 }),
  });
  await harness.delete();

  assert.equal(
    harness.calls.includes("delete_persona"),
    false,
    "cancelling must abort the persona delete, not proceed without the teardown",
  );

  await harness.unmount();
});

test("a partial teardown the user cancelled still blocks the persona delete", async () => {
  // The dangerous shape: some instances are already destroyed, so it is
  // tempting to "finish the job". A provider-deployed instance may be among
  // the survivors, and delete_persona would refuse the cascade anyway.
  const harness = await mountPersonaActions({
    deleteInstances: async () => ({ cancelled: true, deletedCount: 3 }),
  });
  await harness.delete();

  assert.equal(harness.calls.includes("delete_persona"), false);

  await harness.unmount();
});

test("a failed teardown never deletes the persona", async () => {
  const harness = await mountPersonaActions({
    deleteInstances: async () => {
      throw new Error("backend refused");
    },
  });
  // handleDelete reports failures through its error-message state rather than
  // rejecting, so the absence of delete_persona is the only observable proof
  // it aborted.
  await harness.delete();

  assert.equal(
    harness.calls.includes("delete_persona"),
    false,
    "a throw mid-teardown must not fall through to the persona delete",
  );

  await harness.unmount();
});
