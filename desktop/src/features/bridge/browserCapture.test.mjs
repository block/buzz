import assert from "node:assert/strict";
import test from "node:test";
import { makeBrowserDeps } from "./browserHost.ts";

function fakeHost(open, bytes) {
  const calls = [];
  return {
    calls,
    host: {
      rows: () => [],
      subscribe: () => () => {},
      add: () => ({ sid: "x", title: "x" }),
      show: (sid) => calls.push(["show", sid]),
      remove: async () => {},
      isOpen: async () => open,
      screenshot: async () => (calls.push(["screenshot"]), { bytes }),
      grants: async () => [],
      setAgent: async () => {},
      clearAgent: async () => {},
      subscribeGrants: () => () => {},
    },
  };
}

test("screenshot never shows/navigates: Cold is refused with a 'start it first' reason, nothing is captured", async () => {
  const f = fakeHost(false, [1, 2, 3]);
  const deps = makeBrowserDeps(() => f.host, async () => {}, async () => {});
  await assert.rejects(deps.capture("s1"), /Cold - start it first/);
  assert.deepEqual(f.calls, []);
});

test("screenshot of a Live browser snapshots it in place (no show call); an empty capture is an error", async () => {
  const f = fakeHost(true, []);
  const deps = makeBrowserDeps(() => f.host, async () => {}, async () => {});
  await assert.rejects(deps.capture("s1"), /couldn't capture/);
  assert.deepEqual(f.calls, [["screenshot"]]);
});
