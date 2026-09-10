import assert from "node:assert/strict";
import test from "node:test";

const { createBrowserSessionLifecycle } = await import(
  "./useBrowserSession.ts"
);

const BOUNDS = { x: 0, y: 0, width: 100, height: 100 };

/** A resolve/reject pair the test controls explicitly — no timers, no races. */
function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Lets already-queued microtasks (the lifecycle's internal `chain`) drain before an assertion. */
function flushMicrotasks() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/** Records every call and lets the test resolve/reject each one on demand. */
function createFakeInvoke() {
  const calls = [];
  const pending = [];
  const invokeFn = (command, payload) => {
    const entry = deferred();
    calls.push({ command, payload });
    pending.push(entry);
    return entry.promise;
  };
  return { invokeFn, calls, pending };
}

test("a late open still closes once it resolves, even after teardown was requested first", async () => {
  const { invokeFn, calls, pending } = createFakeInvoke();
  const lifecycle = createBrowserSessionLifecycle(invokeFn);

  const openPromise = lifecycle.open("plugin-a", "web", BOUNDS);
  // Teardown is requested while the open's native `plugin_browser_open` call
  // is still in flight — this must not return until the eventually-adopted
  // session is closed, not merely until the raw open settles.
  const closePromise = lifecycle.closeActive();

  await flushMicrotasks();
  assert.equal(
    calls.length,
    1,
    "close must not run ahead of the open it follows",
  );
  assert.equal(calls[0].command, "plugin_browser_open");

  // The native open now resolves late.
  pending[0].resolve({
    sessionId: "s1",
    generation: 1,
    homeUrl: "https://example.com/",
  });
  const opened = await openPromise;
  assert.equal(opened.sessionId, "s1");

  await flushMicrotasks();
  assert.equal(calls.length, 2, "the late-adopted session must be closed");
  assert.equal(calls[1].command, "plugin_browser_close");
  assert.equal(calls[1].payload.sessionId, "s1");
  pending[1].resolve(undefined);

  await closePromise;
});

test("overlapping opens: the second serializes after the first and closes it first", async () => {
  const { invokeFn, calls, pending } = createFakeInvoke();
  const lifecycle = createBrowserSessionLifecycle(invokeFn);

  const openA = lifecycle.open("plugin-a", "web", BOUNDS);
  await flushMicrotasks();
  pending[0].resolve({
    sessionId: "s1",
    generation: 1,
    homeUrl: "https://a.example/",
  });
  await openA;

  // A second open is issued without an explicit close in between.
  const openB = lifecycle.open("plugin-b", "web", BOUNDS);

  // The close of A must be issued before B's own open call.
  await flushMicrotasks();
  assert.equal(calls.length, 2);
  assert.equal(calls[1].command, "plugin_browser_close");
  assert.equal(calls[1].payload.sessionId, "s1");
  pending[1].resolve(undefined);

  await flushMicrotasks();
  assert.equal(calls.length, 3);
  assert.equal(calls[2].command, "plugin_browser_open");
  assert.equal(calls[2].payload.pluginId, "plugin-b");

  pending[2].resolve({
    sessionId: "s2",
    generation: 1,
    homeUrl: "https://b.example/",
  });
  const opened = await openB;
  assert.equal(opened.sessionId, "s2");
});

test("a close failure propagates to the caller, retains the session, and a later close retries it", async () => {
  const { invokeFn, calls, pending } = createFakeInvoke();
  const lifecycle = createBrowserSessionLifecycle(invokeFn);

  const openA = lifecycle.open("plugin-a", "web", BOUNDS);
  await flushMicrotasks();
  pending[0].resolve({
    sessionId: "s1",
    generation: 1,
    homeUrl: "https://a.example/",
  });
  await openA;

  const firstClose = lifecycle.closeActive();
  await flushMicrotasks();
  assert.equal(
    calls.length,
    2,
    "close must have been issued for the active session",
  );
  pending[1].reject(new Error("native close failed"));
  await assert.rejects(firstClose, /native close failed/);

  // The session must not have been forgotten by the failed close — a later
  // close retries the exact same sessionId rather than treating nothing as
  // active and silently leaking the still-live native session.
  const secondClose = lifecycle.closeActive();
  await flushMicrotasks();
  assert.equal(calls.length, 3, "the retained session must be retried");
  assert.equal(calls[2].command, "plugin_browser_close");
  assert.equal(calls[2].payload.sessionId, "s1");
  pending[2].resolve(undefined);
  await secondClose;

  // Once the retry succeeds, the session is finally forgotten — a further
  // close is a true no-op.
  const thirdClose = lifecycle.closeActive();
  await thirdClose;
  assert.equal(calls.length, 3, "no session remained active to close again");
});

test("closing with nothing open or opening resolves immediately with no native call", async () => {
  const { invokeFn, calls } = createFakeInvoke();
  const lifecycle = createBrowserSessionLifecycle(invokeFn);
  await lifecycle.closeActive();
  assert.equal(calls.length, 0);
});
