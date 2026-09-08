import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { JSDOM } from "jsdom";

test("mounted explicit import keeps exact scope/key, clears secret on failure and retries without starting", async () => {
  const dom = new JSDOM("<div id='root'></div>", {
    url: "https://desktop.test",
  });
  for (const key of [
    "window",
    "document",
    "HTMLElement",
    "Element",
    "Node",
    "NodeFilter",
    "MutationObserver",
    "CustomEvent",
    "Event",
    "HTMLInputElement",
  ])
    globalThis[key] =
      key === "window"
        ? dom.window
        : key === "document"
          ? dom.window.document
          : dom.window[key];
  globalThis.getComputedStyle = dom.window.getComputedStyle;
  globalThis.localStorage = dom.window.localStorage;
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  const { ThemeProvider } = await import("@/shared/theme/ThemeProvider");
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const { ExistingAgentDialog } = await import("./AddExistingAgent.tsx");
  const { createRoot } = await import("react-dom/client");
  const { fireEvent } = await import("@testing-library/react");
  const calls = [];
  let reject = true,
    saved = 0;
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      if (command !== "add_existing_agent") return;
      calls.push({ command, args });
      if (reject) throw new Error("synthetic-secret-must-not-render");
    },
  };
  const root = createRoot(document.getElementById("root"));
  const scope = { owner: "owner", community: "wss://one.example" };
  try {
    await React.act(async () =>
      root.render(
        React.createElement(
          ThemeProvider,
          null,
          React.createElement(ExistingAgentDialog, {
            scope,
            onSaved: () => saved++,
          }),
        ),
      ),
    );
    await React.act(async () => document.querySelector("button").click());
    const inputs = document.querySelectorAll("input");
    assert.equal(inputs.length, 2);
    assert.equal(inputs[1].type, "password");
    assert.equal(inputs[1].autocomplete, "off");
    await React.act(async () => {
      fireEvent.change(inputs[0], { target: { value: "exact-public-key" } });
      fireEvent.change(inputs[1], { target: { value: "synthetic-secret" } });
    });
    await React.act(async () =>
      fireEvent.submit(document.querySelector("form")),
    );
    assert.deepEqual(calls, [
      {
        command: "add_existing_agent",
        args: {
          input: {
            ...scope,
            pubkey: "exact-public-key",
            privateKey: "synthetic-secret",
          },
        },
      },
    ]);
    assert.equal(inputs[1].value, "");
    assert.ok(document.querySelector("[role=alert]"));
    assert.ok(!document.body.textContent.includes("synthetic-secret"));
    assert.equal(saved, 0);
    reject = false;
    await React.act(async () =>
      fireEvent.change(inputs[1], { target: { value: "synthetic-secret" } }),
    );
    await React.act(async () =>
      fireEvent.submit(document.querySelector("form")),
    );
    assert.equal(saved, 1);
    assert.equal(calls.length, 2);
    assert.deepEqual(calls[0], calls[1]);
    assert.equal(document.querySelector("input[type=password]"), null);
  } finally {
    await React.act(async () => root.unmount());
    dom.window.close();
  }
});
