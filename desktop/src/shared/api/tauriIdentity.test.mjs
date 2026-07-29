import test from "node:test";
import assert from "node:assert/strict";

// Regression coverage for the standalone-web splash-screen hang: get_identity
// (and its siblings) must never reach invokeTauri when there is no Tauri
// runtime — reaching it produced a raw TypeError deep inside
// @tauri-apps/api/core's invoke(), and combined with the identity query's
// retry policy, left the query stuck on fetchStatus "paused" forever instead
// of settling to "error". These tests assert the guard short-circuits before
// any native call is attempted.
//
// Each test mocks fresh via its own TestContext (`t.mock.module`), which
// auto-restores at test end — a shared/global `mock.module` call here hits
// "module is already mocked" on the second test, since the mock registry is
// process-global and outlives a manual `.restore()` call made mid-run.
//
// The "Tauri is available" happy-path case lives in
// tauriIdentity.available.test.mjs, a separate file — see that file's
// comment for why it can't share a process with these "unavailable" tests.

test("getIdentity rejects without invoking the native bridge when Tauri is unavailable", async (t) => {
  t.mock.module("@tauri-apps/api/core", {
    namedExports: { isTauri: () => false },
  });
  const invokeTauriFn = t.mock.fn();
  t.mock.module("@/shared/api/tauri", {
    namedExports: { invokeTauri: invokeTauriFn },
  });

  const { getIdentity } = await import("@/shared/api/tauriIdentity");

  await assert.rejects(() => getIdentity(), /no native identity backend/);
  assert.equal(
    invokeTauriFn.mock.calls.length,
    0,
    "invokeTauri must not be called when Tauri is unavailable",
  );
});

for (const [name, fn] of [
  ["getNsec", (m) => m.getNsec()],
  ["importIdentity", (m) => m.importIdentity("nsec1test")],
  ["persistCurrentIdentity", (m) => m.persistCurrentIdentity()],
  ["signOut", (m) => m.signOut()],
]) {
  test(`${name} rejects without invoking the native bridge when Tauri is unavailable`, async (t) => {
    t.mock.module("@tauri-apps/api/core", {
      namedExports: { isTauri: () => false },
    });
    const invokeTauriFn = t.mock.fn();
    t.mock.module("@/shared/api/tauri", {
      namedExports: { invokeTauri: invokeTauriFn },
    });

    const module = await import("@/shared/api/tauriIdentity");

    await assert.rejects(() => fn(module));
    assert.equal(
      invokeTauriFn.mock.calls.length,
      0,
      `invokeTauri must not be called by ${name} when Tauri is unavailable`,
    );
  });
}
