import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useHostRegistration } from "./useHostRegistration.ts";
import { hostNativeDrain } from "./hostNativeDrain.ts";
import { hostQueryKey } from "./registration.ts";
import { fixture } from "./hostTestFixtures.mjs";

const flush = () => new Promise((resolve) => setImmediate(resolve));
for (const switchKind of ["identity", "community"]) {
  test(`actual registration hook waits across keyed ${switchKind} + query-provider remount`, async (t) => {
    const saved = Object.fromEntries(
      ["window", "document", "IS_REACT_ACT_ENVIRONMENT"].map((key) => [
        key,
        globalThis[key],
      ]),
    );
    const dom = new JSDOM("<div id='root'></div>", {
      url: "https://fixture.invalid",
    });
    Object.assign(globalThis, {
      window: dom.window,
      document: dom.window.document,
      IS_REACT_ACT_ENVIRONMENT: true,
    });
    const root = createRoot(document.getElementById("root"));
    const calls = [];
    const releases = [];
    const f = fixture();
    dom.window.__TAURI_INTERNALS__ = {
      invoke: (command, args) => {
        assert.equal(
          command,
          "get_local_host",
          "cancelled mounts must not connect or publish",
        );
        calls.push(args.expectedOwner);
        return new Promise((resolve) =>
          releases.push(async () => resolve(await f.bridge.local())),
        );
      },
    };
    t.after(async () => {
      await act(async () => root.unmount());
      for (const release of releases) await release();
      await hostNativeDrain.wait();
      await new Promise((resolve) => setTimeout(resolve, 10));
      dom.window.close();
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete globalThis[key];
        else globalThis[key] = value;
      }
    });
    const first = new QueryClient({
      defaultOptions: { queries: { gcTime: Infinity } },
    });
    const second = new QueryClient({
      defaultOptions: { queries: { gcTime: Infinity } },
    });
    const owner = "a".repeat(64),
      relay = "wss://fixture.invalid";
    const nextOwner = switchKind === "identity" ? "c".repeat(64) : owner;
    const nextRelay =
      switchKind === "community" ? "wss://other.invalid" : relay;
    function Publisher({ owner, relay }) {
      useHostRegistration(owner, relay);
      return null;
    }
    const tree = (key, client, owner, relay) =>
      React.createElement(
        QueryClientProvider,
        { key, client },
        React.createElement(Publisher, { key, owner, relay }),
      );
    await act(async () => {
      root.render(tree("old", first, owner, relay));
      await flush();
    });
    assert.deepEqual(calls, [owner]);
    assert.ok(first.getQueryData(hostQueryKey(relay, owner)));
    await act(async () => {
      root.render(tree("new", second, nextOwner, nextRelay));
      await flush();
    });
    assert.deepEqual(
      calls,
      [owner],
      "new native work cannot overtake old native discovery",
    );
    assert.equal(first.getQueryData(hostQueryKey(relay, owner)), undefined);
    assert.equal(
      second.getQueryData(hostQueryKey(nextRelay, nextOwner)),
      undefined,
    );
    await act(async () => {
      await releases[0]();
      await flush();
    });
    assert.deepEqual(calls, [owner, nextOwner]);
    await act(async () => root.unmount());
    await releases[1]();
    await hostNativeDrain.wait();
    assert.equal(
      second.getQueryData(hostQueryKey(nextRelay, nextOwner)),
      undefined,
    );
    first.clear();
    second.clear();
  });
}
