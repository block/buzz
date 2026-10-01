import assert from "node:assert/strict";
import test from "node:test";

const LOCAL_RELAY = "ws://localhost:3000";
const LEGACY_RELAY = "wss://legacy.example";
const calls = [];
// Mirrors the native admission record, which survives a webview reload.
const removedRelays = new Set([LEGACY_RELAY]);
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "get_legacy_workspace_storage") {
      return Promise.resolve({
        workspaces: JSON.stringify([
          { id: "legacy", name: "Legacy", relayUrl: LEGACY_RELAY },
          { id: "local", name: "Local", relayUrl: LOCAL_RELAY },
        ]),
        activeWorkspaceId: "legacy",
        onboardingCompletions: [],
      });
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

const { migrateLegacyCommunityStorageBeforeRender } = await import(
  "./legacyCommunityStorage.ts"
);

test("a reload whose legacy migration brings back a removed relay re-admits it once", async () => {
  // Only the localhost community is left after the legacy relay was removed.
  localStorage.setItem(
    "buzz-communities",
    JSON.stringify([{ id: "local", name: "Local", relayUrl: LOCAL_RELAY }]),
  );
  localStorage.setItem("buzz-active-community-id", "local");

  await migrateLegacyCommunityStorageBeforeRender();

  assert.match(localStorage.getItem("buzz-communities"), /legacy\.example/);
  assert.equal(removedRelays.has(LEGACY_RELAY), false, "legacy relay admitted");
  // The unchanged localhost community is not re-admitted.
  assert.equal(calls.filter((c) => c === "readd_community_relay").length, 1);
});
