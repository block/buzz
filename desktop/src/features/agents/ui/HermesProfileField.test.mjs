import assert from "node:assert/strict";
import { after, before, describe, it } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;

const INVENTORY = {
  activeProfile: "default",
  profiles: [
    {
      name: "default",
      displayName: "Lenny",
      description: "",
      descriptionAuto: false,
      isDefault: true,
      active: true,
      model: "gpt-5.6-sol",
      provider: "openai-codex",
      gatewayRunning: true,
      alias: null,
      distribution: null,
    },
    {
      name: "jake",
      displayName: "Jake",
      description: "Implementation agent",
      descriptionAuto: false,
      isDefault: false,
      active: false,
      model: "x-ai/grok-4.6",
      provider: "openrouter",
      gatewayRunning: false,
      alias: "jake",
      distribution: null,
    },
  ],
};

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command) => {
    if (command === "discover_hermes_profiles") {
      return Promise.resolve(INVENTORY);
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  },
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

let React,
  act,
  createRoot,
  QueryClient,
  QueryClientProvider,
  HermesProfileField;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ HermesProfileField } = await import("./HermesProfileField.tsx"));
});

after(() => dom.window.close());

async function mount() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const selected = [];

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(HermesProfileField, {
          disabled: false,
          value: "",
          onValueChange: (value) => selected.push(value),
        }),
      ),
    );
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
  return { client, container, root, selected };
}

describe("HermesProfileField", () => {
  it("shows the real inventory, disables gateway-owned profiles, and selects the first available profile", async () => {
    const view = await mount();
    const select = view.container.querySelector(
      '[data-testid="hermes-profile-select"]',
    );
    assert.ok(select);
    assert.equal(select.options.length, 2);
    assert.equal(select.options[0].value, "default");
    assert.equal(select.options[0].disabled, true);
    assert.match(select.options[0].textContent, /in use by gateway/);
    assert.equal(select.options[1].value, "jake");
    assert.equal(select.options[1].disabled, false);
    assert.match(select.options[1].textContent, /x-ai\/grok-4\.6/);
    assert.deepEqual(view.selected, ["jake"]);
    assert.equal(view.container.textContent.includes("/Users/"), false);

    await act(async () => view.root.unmount());
    view.client.clear();
    view.container.remove();
  });
});
