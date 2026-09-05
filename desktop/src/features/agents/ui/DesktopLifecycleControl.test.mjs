import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  DesktopLifecycleControl,
  DesktopLifecycleReceiver,
} from "./DesktopLifecycleControl.tsx";
import { toast } from "sonner";
import { relayClient } from "../../../shared/api/relayClient.ts";

test("mounted Start exposes unavailable provisioning and exact retry; Restart resolves source", async () => {
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
  const scope = { owner: "owner", community: "wss://one.example" };
  const originals = {
    fetch: relayClient.fetchEvents,
    publish: relayClient.publishEvent,
  };
  const prepared = [],
    sent = [];
  let stop = "failed";
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      assert.equal(args.owner, scope.owner);
      assert.equal(args.community, scope.community);
      if (command === "observe_desktop_placement") return;
      if (command === "read_desktop_placement") return ["source", "selection"];
      if (
        command === "prepare_desktop_lifecycle" ||
        command === "prepare_desktop_stop"
      ) {
        const request = {
          id: `request-${prepared.length}`,
          kind: command.endsWith("_stop") ? 50180 : 50182,
          ...args,
        };
        prepared.push(request);
        return request;
      }
      if (command === "read_desktop_lifecycle_results")
        return args.request.action === "status"
          ? "running"
          : "provisioning_unavailable";
      if (command === "read_desktop_stop_results") return stop;
      throw Error(command);
    },
  };
  relayClient.fetchEvents = async () => [];
  relayClient.publishEvent = async (event, _timeout, _failure, check) => {
    check();
    sent.push(event);
  };
  const root = createRoot(document.getElementById("root"));
  const click = (text) =>
    React.act(async () =>
      [...document.querySelectorAll("button")]
        .find((b) => b.textContent === text)
        .click(),
    );
  const select = (label, value) =>
    React.act(async () => {
      const element = document.querySelector(`select[aria-label="${label}"]`);
      element.value = value;
      element.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    });
  try {
    await React.act(async () =>
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(DesktopLifecycleControl, {
            scope,
            desktops: [
              { id: "source", name: "Source" },
              { id: "destination", name: "Destination" },
            ],
          }),
        ),
      ),
    );
    assert.doesNotMatch(document.body.textContent, /Foreign agent/);
    await select("Agent to place", "agent");
    await select("Destination Desktop", "destination");
    await click("Start on destination");
    assert.match(
      document.body.textContent,
      /keyless launch provisioning is unavailable/,
    );
    assert.equal(prepared[0].desktop, "destination");
    await click("Retry same request");
    assert.equal(prepared.length, 1);
    assert.equal(sent[0], sent[1]);
    await click("Restart on current Desktop");
    const restart = prepared.at(-1);
    assert.equal(
      restart.desktop,
      "source",
      "destination picker must not redirect Restart",
    );
    assert.equal(restart.action, "restart");
    assert.equal(restart.observed, prepared.at(-2).id);
    await click("Move to destination");
    assert.match(document.body.textContent, /destination was not started/);
    const count = prepared.length;
    stop = "stopped";
    await React.act(async () => {});
    assert.equal(prepared.length, count, "late Stop cannot resume failed Move");
    assert.doesNotMatch(document.body.textContent, /Retry same request/);
  } finally {
    await React.act(async () => root.unmount());
    client.clear();
    relayClient.fetchEvents = originals.fetch;
    relayClient.publishEvent = originals.publish;
    dom.window.close();
  }
});

test("terminal receiver failure is a scope-owned notification, not pre-shell layout", async () => {
  const originalRaf = globalThis.requestAnimationFrame;
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
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
  const originals = {
    fetch: relayClient.fetchEvents,
    subscribe: relayClient.subscribeLive,
  };
  let readiness;
  let closed = 0;
  let rejectLate;
  let delayed = false;
  let subscribed = 0;
  relayClient.fetchEvents = async () => {
    if (delayed)
      return new Promise((_, reject) => {
        rejectLate = reject;
      });
    return [];
  };
  relayClient.subscribeLive = async (
    _filter,
    _event,
    _onReadiness,
    _timeout,
    options,
  ) => {
    subscribed++;
    readiness = options.onState;
    readiness("eose");
    return () => {
      closed++;
    };
  };
  window.__TAURI_INTERNALS__ = {
    invoke: async () => {},
  };
  const root = createRoot(document.getElementById("root"));
  const scope = { owner: "owner", community: "wss://one.example" };
  const warnings = () =>
    toast
      .getToasts()
      .filter((t) => String(t.title).startsWith("Desktop lifecycle"));
  try {
    await React.act(async () =>
      root.render(React.createElement(DesktopLifecycleReceiver, { scope })),
    );
    assert.equal(
      document.getElementById("root").childElementCount,
      0,
      "startup must not render in-flow failure UI",
    );
    assert.equal(warnings().length, 0);
    await React.act(async () =>
      readiness("closed", { classification: "terminal", retryAfterMs: 0 }),
    );
    assert.equal(warnings().length, 1);
    assert.equal(
      warnings()[0].title,
      "Desktop lifecycle receiver subscription closed. Retry the receiver to accept new requests.",
    );
    assert.equal(warnings()[0].duration, Infinity);
    assert.equal(warnings()[0].closeButton, true);
    readiness("closed", { classification: "terminal", retryAfterMs: 0 });
    assert.equal(
      warnings().length,
      1,
      "repeated failures update one notification",
    );
    const retryAction = warnings()[0].action;
    assert.equal(retryAction.label, "Retry receiver");
    await React.act(async () => retryAction.onClick());
    assert.equal(subscribed, 2, "explicit recovery starts a new live receiver");
    assert.equal(
      warnings().length,
      0,
      "successful recovery removes its warning",
    );
    await React.act(async () =>
      root.render(
        React.createElement(DesktopLifecycleReceiver, { scope: null }),
      ),
    );
    assert.equal(warnings().length, 0, "leaving the scope removes its warning");
    readiness("closed", { classification: "terminal", retryAfterMs: 0 });
    assert.equal(
      warnings().length,
      0,
      "retired receiver cannot notify another scope",
    );
    delayed = true;
    await React.act(async () =>
      root.render(React.createElement(DesktopLifecycleReceiver, { scope })),
    );
    assert.equal(typeof rejectLate, "function");
    await React.act(async () => root.unmount());
    await React.act(async () => rejectLate(new Error("late failure")));
    assert.equal(
      warnings().length,
      0,
      "late startup rejection must not recreate the warning",
    );
    assert.equal(
      closed,
      3,
      "failed, recovered and retired subscriptions are released",
    );
  } finally {
    await React.act(async () => root.unmount());
    relayClient.fetchEvents = originals.fetch;
    relayClient.subscribeLive = originals.subscribe;
    for (const warning of warnings()) toast.dismiss(warning.id);
    globalThis.requestAnimationFrame = originalRaf;
    dom.window.close();
  }
});
