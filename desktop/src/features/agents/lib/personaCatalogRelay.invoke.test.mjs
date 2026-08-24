import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { fetchPersonaCatalogPublications } from "./personaCatalogRelay.ts";

function installTauriInvoke(handler) {
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = { invoke: handler };
}

test("persona catalog wrapper invokes the one native active-scope command", async (t) => {
  const prior = globalThis.window;
  t.after(() => {
    mock.restoreAll();
    globalThis.window = prior;
  });
  const expected = [{ eventId: "event-1", ownerPubkey: "alice" }];
  const calls = [];
  installTauriInvoke((command, args) => {
    calls.push([command, args]);
    return Promise.resolve(expected);
  });

  assert.deepEqual(await fetchPersonaCatalogPublications(), expected);
  assert.deepEqual(calls, [["fetch_persona_catalog", {}]]);
});
