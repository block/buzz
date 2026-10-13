import assert from "node:assert/strict";
import { test } from "node:test";

const mod = await import("./nativeWebviewModalPark.ts");

test("before-park hooks run on the first acquire and see park/unpark", async () => {
  const events = [];
  const offHook = mod.registerBeforeParkHook(() => {
    events.push("stand-in");
  });
  const offListen = mod.subscribeNativeWebviewModalPark((parked) =>
    events.push(parked ? "parked" : "restored"),
  );
  mod.acquireNativeWebviewModalPark();
  mod.acquireNativeWebviewModalPark(); // nested menu: no second stand-in
  mod.releaseNativeWebviewModalPark();
  mod.releaseNativeWebviewModalPark();
  assert.deepEqual(events, ["stand-in", "parked", "restored"]);
  offHook();
  offListen();
  mod.resetNativeWebviewModalPark();
});

test("a slow capture hook never blocks the park beyond the wait cap", async () => {
  let resolved = false;
  const off = mod.registerBeforeParkHook(
    () => new Promise((resolve) => setTimeout(() => { resolved = true; resolve(); }, 5_000)),
  );
  const startedAt = Date.now();
  mod.acquireNativeWebviewModalPark();
  await new Promise((r) => setTimeout(r, mod.STAND_IN_CAPTURE_WAIT_MS + 50));
  assert.equal(resolved, false);
  assert.ok(Date.now() - startedAt < 5_000);
  mod.releaseNativeWebviewModalPark();
  off();
  mod.resetNativeWebviewModalPark();
});
