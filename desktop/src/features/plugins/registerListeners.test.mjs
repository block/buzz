import assert from "node:assert/strict";
import test from "node:test";

const { registerListeners } = await import("./registerListeners.ts");

test("one rejected registration doesn't lose the ones that succeeded", async () => {
  const unlistenA = () => {};
  const unlistenC = () => {};
  const cause = new Error("registration failed");
  const { succeeded, failedReasons } = await registerListeners([
    async () => unlistenA,
    async () => {
      throw cause;
    },
    async () => unlistenC,
  ]);
  assert.deepEqual(succeeded, [unlistenA, unlistenC]);
  assert.deepEqual(failedReasons, [cause]);
});

test("a registration that resolves after a sibling rejects still ends up in succeeded — allSettled, not all", async () => {
  const cause = new Error("registration failed");
  let resolveLate;
  const late = new Promise((resolve) => {
    resolveLate = resolve;
  });
  const unlistenLate = () => {};

  const pending = registerListeners([
    async () => {
      throw cause;
    },
    () => late,
  ]);
  // The rejection settles first; the second registration is still pending.
  await Promise.resolve();
  resolveLate(unlistenLate);

  const { succeeded, failedReasons } = await pending;
  assert.deepEqual(succeeded, [unlistenLate]);
  assert.deepEqual(failedReasons, [cause]);
});

test("all registrations succeeding reports no failures", async () => {
  const unlistenA = () => {};
  const unlistenB = () => {};
  const { succeeded, failedReasons } = await registerListeners([
    async () => unlistenA,
    async () => unlistenB,
  ]);
  assert.deepEqual(succeeded, [unlistenA, unlistenB]);
  assert.deepEqual(failedReasons, []);
});
