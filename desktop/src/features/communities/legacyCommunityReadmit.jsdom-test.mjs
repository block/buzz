import assert from "node:assert/strict";
import test from "node:test";

const LOCAL_RELAY = "ws://localhost:3000";
const LEGACY_RELAY = "wss://legacy.example";
const calls = [];
// Mirrors the native admission record, which survives a webview reload.
const removedRelays = new Set();
let legacyWorkspaces;
let onboardingCompletions;
const tauriMock = {
  invoke(command, args) {
    calls.push(command);
    if (command === "get_legacy_workspace_storage") {
      return Promise.resolve({
        workspaces: JSON.stringify(legacyWorkspaces),
        activeWorkspaceId: "legacy",
        onboardingCompletions,
      });
    }
    if (command === "readd_community_relay") {
      // Mirrors only native's credential and fragment rejection, not full validation.
      const url = new URL(args.relayUrl);
      if (url.username || url.password || args.relayUrl.includes("#")) {
        return Promise.reject(new Error("invalid relay url"));
      }
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

const COMPLETION_KEY_PREFIX = "buzz-onboarding-complete.v1:";

// Only the localhost community is left after the legacy relay was removed.
async function reloadWithLegacy({
  workspaces,
  completions = [],
  savedLocalRelay = LOCAL_RELAY,
}) {
  localStorage.clear();
  localStorage.setItem(
    "buzz-communities",
    JSON.stringify([{ id: "local", name: "Local", relayUrl: savedLocalRelay }]),
  );
  localStorage.setItem("buzz-active-community-id", "local");
  removedRelays.clear();
  removedRelays.add(LEGACY_RELAY);
  legacyWorkspaces = workspaces;
  onboardingCompletions = completions;
  calls.length = 0;
  await migrateLegacyCommunityStorageBeforeRender();
  return calls.filter((c) => c === "readd_community_relay").length;
}

const legacy = { id: "legacy", name: "Legacy", relayUrl: LEGACY_RELAY };
const local = { id: "local", name: "Local", relayUrl: LOCAL_RELAY };

test("a reload whose legacy migration brings back a removed relay re-admits it once", async () => {
  const readds = await reloadWithLegacy({ workspaces: [legacy, local] });
  assert.match(localStorage.getItem("buzz-communities"), /legacy\.example/);
  assert.equal(removedRelays.has(LEGACY_RELAY), false, "legacy relay admitted");
  // The unchanged localhost community is not re-admitted.
  assert.equal(readds, 1);
});

test("a failed onboarding-completion write after the list is saved still re-admits the relay once", async () => {
  const setItem = Storage.prototype.setItem;
  Storage.prototype.setItem = function (key, value) {
    if (key.startsWith(COMPLETION_KEY_PREFIX)) {
      throw new DOMException("full", "QuotaExceededError");
    }
    return setItem.call(this, key, value);
  };
  try {
    const readds = await reloadWithLegacy({
      workspaces: [legacy],
      completions: [{ pubkey: "ab".repeat(32), value: "1" }],
    });
    assert.equal(localStorage.getItem("buzz-active-community-id"), "legacy");
    assert.equal(
      removedRelays.has(LEGACY_RELAY),
      false,
      "legacy relay admitted",
    );
    assert.equal(readds, 1);
  } finally {
    Storage.prototype.setItem = setItem;
  }
});

for (const alias of [`${LEGACY_RELAY}/#old`, "wss://user@legacy.example"]) {
  test(`an invalid alias ${alias} listed before the valid relay still re-admits it`, async () => {
    await reloadWithLegacy({
      workspaces: [{ ...legacy, id: "alias", relayUrl: alias }, legacy],
    });
    assert.equal(
      removedRelays.has(LEGACY_RELAY),
      false,
      "legacy relay admitted",
    );
  });
}

test("the same relay URL listed twice is re-admitted once", async () => {
  const readds = await reloadWithLegacy({ workspaces: [legacy, legacy] });
  assert.equal(readds, 1);
});
