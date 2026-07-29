import test from "node:test";
import assert from "node:assert/strict";

// Split into its own file (not merged with tauriIdentity.test.mjs) because
// node's per-process ESM module cache means mixing an "isTauri: () => true"
// mock with an "isTauri: () => false" mock in the same test file causes
// later imports of tauriIdentity.ts to return the first test's cached module
// instance rather than re-evaluating against the new mock. node --test runs
// each matched file in its own process, which sidesteps that entirely.

test("getIdentity calls the native bridge when Tauri is available", async (t) => {
  t.mock.module("@tauri-apps/api/core", {
    namedExports: { isTauri: () => true },
  });
  const invokeTauriFn = t.mock.fn(async () => ({
    pubkey: "abc",
    display_name: "Test",
  }));
  t.mock.module("@/shared/api/tauri", {
    namedExports: { invokeTauri: invokeTauriFn },
  });

  const { getIdentity } = await import("@/shared/api/tauriIdentity");

  const identity = await getIdentity();
  assert.equal(identity.pubkey, "abc");
  assert.equal(invokeTauriFn.mock.calls.length, 1);
  assert.equal(invokeTauriFn.mock.calls[0].arguments[0], "get_identity");
});
