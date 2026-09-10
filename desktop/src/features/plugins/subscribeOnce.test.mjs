import assert from "node:assert/strict";
import { after, before, test } from "node:test";

let originalWindow;
before(() => {
  originalWindow = globalThis.window;
  // subscribeOnce.ts calls window.setTimeout/clearTimeout — in the real
  // Tauri webview `window` is the global itself, so aliasing it here gives
  // the real timer functions without pulling in jsdom for this file.
  globalThis.window = globalThis;
});
after(() => {
  globalThis.window = originalWindow;
});

const { subscribeOnce } = await import("./subscribeOnce.ts");

test("a registration failure rejects both ready and result, and does not leave the timeout pending", async () => {
  const cause = new Error("registration failed");
  const failingListen = async () => {
    throw cause;
  };
  const { ready, result } = subscribeOnce(
    "some-event",
    () => true,
    // Long enough that a passing test proves rejection came from the
    // registration failure, not from this timeout firing.
    50_000,
    failingListen,
  );
  await assert.rejects(ready, (err) => err === cause);
  await assert.rejects(result, (err) => err === cause);
});

test("a match that arrives before registration resolves disposes the late unlisten instead of leaking it", async () => {
  // Tauri wires the event callback synchronously, ahead of the IPC
  // round-trip its own returned promise tracks — so a fast native event can
  // fire and resolve `result` before `listenFn`'s promise ever settles.
  let disposed = false;
  const fakeListen = async (_eventName, onEvent) => {
    onEvent({ payload: { ok: true } });
    return () => {
      disposed = true;
    };
  };
  const { ready, result } = subscribeOnce(
    "some-event",
    (payload) => payload.ok === true,
    5_000,
    fakeListen,
  );
  assert.deepEqual(await result, { ok: true });
  await ready;
  assert.equal(
    disposed,
    true,
    "the late-registered unlisten must be disposed, not leaked",
  );
});

test("a successful registration resolves ready, and result resolves once a matching event arrives", async () => {
  let handler;
  const fakeListen = async (_eventName, onEvent) => {
    handler = onEvent;
    return () => {};
  };
  const { ready, result } = subscribeOnce(
    "some-event",
    (payload) => payload.ok === true,
    5_000,
    fakeListen,
  );
  await ready;
  handler({ payload: { ok: false } }); // non-matching, ignored
  handler({ payload: { ok: true } });
  assert.deepEqual(await result, { ok: true });
});
