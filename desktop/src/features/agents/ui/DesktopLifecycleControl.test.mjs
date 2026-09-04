import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { JSDOM } from "jsdom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { DesktopLifecycleControl } from "./DesktopLifecycleControl.tsx";
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
