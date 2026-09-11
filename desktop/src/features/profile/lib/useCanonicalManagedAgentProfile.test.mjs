import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
let renderHook;
let cleanup;
let createElement;
let QueryClient;
let QueryClientProvider;
let useCanonicalManagedAgentProfile;
const clients = [];
const A = "a".repeat(64);
const B = "b".repeat(64);
const sibling = {
  pubkey: B,
  personaId: "shared-persona",
  status: "running",
  name: "Local B",
};

before(async () => {
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  ({ renderHook, cleanup } = await import("@testing-library/react"));
  ({ createElement } = await import("react"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ useCanonicalManagedAgentProfile } = await import(
    "./useCanonicalManagedAgentProfile.ts"
  ));
});
afterEach(() => {
  cleanup();
  for (const client of clients.splice(0)) client.clear();
});
after(() => dom.window.close());

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  client.setQueryData(["identity"], { pubkey: "c".repeat(64) });
  client.setQueryData(["archivedIdentities"], { archived: [] });
  clients.push(client);
  return ({ children }) =>
    createElement(QueryClientProvider, { client }, children);
}

for (const managedAgents of [[], [sibling]]) {
  test(`explicit remote A cannot borrow persona P or its ${managedAgents.length} local instances`, () => {
    const { result, rerender } = renderHook(
      (props) => useCanonicalManagedAgentProfile(props),
      {
        wrapper: wrapper(),
        initialProps: { managedAgents, personaId: "shared-persona", pubkey: A },
      },
    );
    assert.equal(result.current.managedAgent, undefined);
    assert.equal(result.current.linkedPersonaId, undefined);
    assert.deepEqual(result.current.instanceBuckets, {
      live: [],
      archived: [],
    });
    // Only deliberately navigating to the persona may choose B or offer Start.
    rerender({ managedAgents, personaId: "shared-persona", pubkey: undefined });
    assert.equal(result.current.linkedPersonaId, "shared-persona");
    assert.equal(result.current.managedAgent, managedAgents[0]);
    // Returning to the explicit key cannot retain the persona representative.
    rerender({ managedAgents, personaId: "shared-persona", pubkey: A });
    assert.equal(result.current.managedAgent, undefined);
    assert.equal(result.current.linkedPersonaId, undefined);
  });
}

test("explicit local instance uses only its own definition link, with normalized key matching", () => {
  const { result } = renderHook(
    () =>
      useCanonicalManagedAgentProfile({
        managedAgents: [sibling],
        personaId: "unrelated-persona",
        pubkey: B.toUpperCase(),
      }),
    { wrapper: wrapper() },
  );
  assert.equal(result.current.managedAgent, sibling);
  assert.equal(result.current.linkedPersonaId, sibling.personaId);
});
