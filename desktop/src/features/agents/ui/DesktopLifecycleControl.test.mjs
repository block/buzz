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

function probeResult(request) {
  if (request.action !== "catalog" && request.action !== "preflight")
    return null;
  const id = request.cursor ? "other" : "config";
  return {
    outcome: "ready",
    observation: {
      valid_until: Math.floor(Date.now() / 1000) + 30,
      catalog:
        request.action === "catalog"
          ? {
              entry: {
                configuration: { id, revision: "revision-1" },
                name: id === "config" ? "Everyday" : "Alternative",
                host: request.desktop,
                runtime: "fixture-harness",
                model: id === "config" ? "Model A" : "Model B",
                provider: null,
                eligible: true,
              },
              next: request.cursor ? null : "config",
            }
          : null,
    },
  };
}

test("mounted picker expires without refresh, reports actual ref, and preserves exact retry", async (t) => {
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
        return (
          probeResult(args.request) ?? {
            outcome:
              args.request.action === "status"
                ? "running"
                : "provisioning_unavailable",
            observation: {
              running_configuration: {
                id: "actual-config",
                revision: "actual-revision",
              },
            },
          }
        );
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
    await select("Runtime configuration", "destination:config:revision-1");
    assert.match(document.body.textContent, /Model A/);
    assert.match(document.body.textContent, /Model B/);
    assert.match(
      document.querySelector('[aria-label="Actual running configuration"]')
        .textContent,
      /actual-config, revision actual-revision/,
    );
    assert.doesNotMatch(
      document.querySelector('[aria-label="Actual running configuration"]')
        .textContent,
      /revision-1/,
    );
    await click("Start on destination");
    assert.match(
      document.body.textContent,
      /keyless launch provisioning is unavailable/,
    );
    assert.equal(
      prepared.find((r) => r.action === "start").desktop,
      "destination",
    );
    await click("Retry same request");
    assert.equal(prepared.filter((r) => r.action === "start").length, 1);
    const starts = sent.filter((r) => r.action === "start");
    assert.equal(starts[0], starts[1]);
    await select("Runtime configuration", "source:other:revision-1");
    await click("Switch runtime configuration");
    assert.match(document.body.textContent, /destination was not started/);
    const count = prepared.length;
    stop = "stopped";
    await React.act(async () => {});
    assert.equal(prepared.length, count, "late Stop cannot resume failed Move");
    assert.doesNotMatch(document.body.textContent, /Retry same request/);
    t.mock.timers.enable({ apis: ["Date", "setTimeout"], now: Date.now() });
    await click("Refresh configurations");
    await select("Runtime configuration", "source:config:revision-1");
    await React.act(async () => t.mock.timers.tick(31_000));
    assert.equal(
      document.querySelector('[aria-label="Runtime configuration"]').options
        .length,
      1,
    );
    assert.equal(
      [...document.querySelectorAll("button")].find(
        (b) => b.textContent === "Start on destination",
      ).disabled,
      true,
    );
    assert.equal(
      [...document.querySelectorAll("button")].find(
        (b) => b.textContent === "Switch runtime configuration",
      ).disabled,
      true,
    );
    assert.equal(
      prepared.filter((r) => r.action === "start").length,
      1,
      "expiry never dispatches Start",
    );
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

for (const interruption of ["cancel", "account", "community", "unmount"]) {
  test(`mounted ${interruption} retires a pending Stop and never starts on late confirmation`, async () => {
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
    const query = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: Infinity } },
    });
    query.setQueryData(
      ["relay-agents"],
      [{ pubkey: "agent", name: "Agent", ownerPubkey: "owner" }],
    );
    const original = {
      fetch: relayClient.fetchEvents,
      publish: relayClient.publishEvent,
    };
    const prepared = [];
    let confirmStop;
    window.__TAURI_INTERNALS__ = {
      invoke: async (command, args) => {
        if (command === "observe_desktop_placement") return;
        if (command === "read_desktop_placement")
          return ["source", "selection"];
        if (command.startsWith("prepare_desktop_")) {
          const request = {
            id: `request-${prepared.length}`,
            kind: command.endsWith("_stop") ? 50180 : 50182,
            ...args,
          };
          prepared.push(request);
          return request;
        }
        if (command === "read_desktop_lifecycle_results")
          return probeResult(args.request) ?? { outcome: "running" };
        if (command === "read_desktop_stop_results")
          return new Promise((resolve) => {
            confirmStop = resolve;
          });
        throw Error(command);
      },
    };
    relayClient.fetchEvents = async () => [];
    relayClient.publishEvent = async (_event, _timeout, _failure, check) =>
      check();
    const root = createRoot(document.getElementById("root"));
    const scope = { owner: "owner", community: "wss://one.example" };
    const render = (nextScope) =>
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: query },
          React.createElement(DesktopLifecycleControl, {
            scope: nextScope,
            desktops: [
              { id: "source", name: "Source" },
              { id: "target", name: "Target" },
            ],
          }),
        ),
      );
    const click = async (text) =>
      React.act(async () =>
        [...document.querySelectorAll("button")]
          .find((b) => b.textContent === text)
          .click(),
      );
    const select = async (label, value) =>
      React.act(async () => {
        const element = document.querySelector(`select[aria-label="${label}"]`);
        element.value = value;
        element.dispatchEvent(
          new dom.window.Event("change", { bubbles: true }),
        );
      });
    try {
      await React.act(async () => render(scope));
      await select("Agent to place", "agent");
      await select("Runtime configuration", "target:config:revision-1");
      await click("Switch runtime configuration");
      assert.equal(typeof confirmStop, "function");
      if (interruption === "cancel") await click("Cancel waiting");
      else if (interruption === "unmount")
        await React.act(async () => root.unmount());
      else
        await React.act(async () =>
          render({
            ...scope,
            [interruption === "account" ? "owner" : "community"]: "changed",
          }),
        );
      await React.act(async () => confirmStop("stopped"));
      assert.equal(prepared.filter((r) => r.action === "start").length, 0);
      assert.doesNotMatch(
        document.body.textContent,
        /Retry same request|Source Stop confirmed/,
      );
      if (interruption === "cancel")
        assert.match(
          document.body.textContent,
          /Dispatched operations may still finish/,
        );
      if (interruption === "account" || interruption === "community")
        assert.equal(
          document.querySelector('select[aria-label="Agent to place"]').value,
          "",
        );
    } finally {
      await React.act(async () => root.unmount());
      query.clear();
      relayClient.fetchEvents = original.fetch;
      relayClient.publishEvent = original.publish;
      dom.window.close();
    }
  });
}
