import assert from "node:assert/strict";
import test from "node:test";

const RELAY_A = "wss://a.example";
const RELAY_B = "wss://b.example";
const calls = [];
// Mirrors the native admission record: removed relays refuse local starts.
const removedRelays = new Set();
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "remove_community_relay") {
      removedRelays.add(args.relayUrl);
      return Promise.resolve();
    }
    if (command === "readd_community_relay") {
      removedRelays.delete(args.relayUrl);
      return Promise.resolve();
    }
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { CommunitiesProvider, useCommunities } = await import(
  "./useCommunities.tsx"
);
const { saveActiveCommunityId, saveCommunities } = await import(
  "./communityStorage.ts"
);
const { refuseRelayAdmission } = await import(
  "../agents/managedAgentRelayCleanup.ts"
);

test("editing a community onto a removed relay re-admits that relay once", async () => {
  // B was removed from this device earlier; only A is still saved.
  saveCommunities([{ id: "a", name: "A", relayUrl: RELAY_A }]);
  saveActiveCommunityId("a");
  await refuseRelayAdmission(RELAY_B);
  assert.ok(removedRelays.has(RELAY_B));

  const latest = {};
  function Harness() {
    Object.assign(latest, useCommunities());
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(
      React.createElement(
        CommunitiesProvider,
        null,
        React.createElement(Harness),
      ),
    );
  });

  calls.length = 0;
  let result;
  await act(async () => {
    result = latest.updateCommunity("a", { relayUrl: RELAY_B });
  });
  // Any reconcile queued after the edit waits on the admission write queue.
  await refuseRelayAdmission("wss://unrelated.example");

  assert.equal(result.kind, "updated");
  assert.equal(removedRelays.has(RELAY_B), false, "B admits local starts");
  assert.equal(calls.filter((c) => c === "readd_community_relay").length, 1);

  // A name-only edit does not re-admit.
  calls.length = 0;
  await act(async () => {
    latest.updateCommunity("a", { name: "Renamed" });
  });
  assert.equal(calls.includes("readd_community_relay"), false);
  await act(async () => root.unmount());
});
