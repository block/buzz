import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  DesktopStopControl,
  DesktopStopReceiver,
} from "./DesktopStopControl.tsx";
import { relayClient } from "../../../shared/api/relayClient.ts";

test("mounted Stop waits for a correlated result and retries identical bytes without replay", async () => {
  const dom = new JSDOM("<div id='root'></div>", {
    url: "https://desktop.test",
  });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const { createRoot } = await import("react-dom/client");
  const scope = { owner: "owner", community: "wss://one.example" };
  const request = { id: "request", kind: 50180, tags: [["d", "desktop"]] };
  const result = { id: "result", kind: 50181, tags: [["e", request.id]] };
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(
    ["relay-agents"],
    [
      { pubkey: "agent", name: "Owned agent", ownerPubkey: "owner" },
      { pubkey: "foreign", name: "Foreign agent", ownerPubkey: "other" },
    ],
  );
  const original = {
    fetch: relayClient.fetchEvents,
    publish: relayClient.publishEvent,
    subscribe: relayClient.subscribeLive,
  };
  let live, release;
  let receiveCalls = 0,
    prepared = 0,
    closed = 0;
  const sent = [];
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      assert.equal(args.owner, scope.owner);
      assert.equal(args.community, scope.community);
      if (command === "prepare_desktop_stop") {
        prepared++;
        assert.equal(args.desktop, "desktop");
        assert.equal(args.agent, "agent");
        return request;
      }
      if (command === "receive_desktop_stop") {
        receiveCalls++;
        return result;
      }
      assert.equal(command, "read_desktop_stop_results");
      return "stopped";
    },
  };
  relayClient.subscribeLive = async (filter, callback) => {
    assert.deepEqual(filter, {
      kinds: [50180],
      authors: [scope.owner],
      limit: 0,
    });
    live = callback;
    return () => {
      closed++;
      live = undefined;
    };
  };
  relayClient.publishEvent = async (event) => {
    sent.push(event);
    live?.(event);
  };
  relayClient.fetchEvents = async (filter) => {
    assert.deepEqual(filter, {
      kinds: [50181],
      authors: [scope.owner],
      "#e": [request.id],
      limit: 16,
    });
    return new Promise((resolve) => {
      release = () => resolve([result]);
    });
  };
  const root = createRoot(document.getElementById("root"));
  const render = (receiver) =>
    React.act(async () =>
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(
            React.Fragment,
            null,
            receiver
              ? React.createElement(DesktopStopReceiver, { scope })
              : null,
            React.createElement(DesktopStopControl, {
              scope,
              desktop: { id: "desktop", name: "Workshop" },
            }),
          ),
        ),
      ),
    );
  const click = (text) =>
    React.act(async () =>
      [...document.querySelectorAll("button")]
        .find((b) => b.textContent === text)
        .click(),
    );
  try {
    await render(false);
    assert.doesNotMatch(document.body.textContent, /Foreign agent/);
    const select = document.querySelector("select");
    await React.act(async () => {
      select.value = "agent";
      select.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    });
    await click("Stop on Workshop");
    assert.equal(prepared, 1);
    assert.match(document.body.textContent, /Waiting for this Desktop/);
    assert.doesNotMatch(document.body.textContent, /Stop confirmed/);
    assert.equal(receiveCalls, 0, "absent receiver has not stopped anything");
    await React.act(async () => release());
    assert.match(document.body.textContent, /Stop confirmed by Workshop/);
    await render(true);
    assert.equal(receiveCalls, 0, "mount cannot replay stored Stop");
    // The mounted receiver returns a saved native result, while the sender
    // explicitly retries the exact prepared request rather than signing anew.
    relayClient.publishEvent = async (event) => {
      sent.push(event);
      if (event.kind === 50180) live?.(event);
    };
    await click("Retry same Stop");
    await React.act(async () => release());
    assert.equal(prepared, 1);
    assert.equal(receiveCalls, 1);
    assert.ok(sent.filter((e) => e.kind === 50180).every((e) => e === request));
  } finally {
    await React.act(async () => root.unmount());
    assert.equal(closed, 1);
    client.clear();
    relayClient.fetchEvents = original.fetch;
    relayClient.publishEvent = original.publish;
    relayClient.subscribeLive = original.subscribe;
    dom.window.close();
  }
});
