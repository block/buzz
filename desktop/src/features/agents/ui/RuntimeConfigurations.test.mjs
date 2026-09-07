import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { JSDOM } from "jsdom";
import { RuntimeConfigurationEditor } from "./RuntimeConfigurations.tsx";

const reference = (id, revision = "old") => ({ id, revision });
const entry = (id) => ({
  ...reference(id),
  name: id,
  host: "host",
  runtime: "buzz-agent",
  model: `model-${id}`,
  provider: "openai",
  workspace: null,
  credentialRefs: {},
});
const initial = () => ({
  configurations: {
    selected: "A",
    entries: [entry("A"), entry("B"), entry("unavailable")],
  },
  catalog: ["A", "B", "unavailable"].map((id) => ({
    configuration: reference(id),
    name: id,
    runtime: "buzz-agent",
    model: `model-${id}`,
    eligible: id !== "unavailable",
  })),
  host: "host",
  updatedAt: "before",
  running: reference("A"),
});

test("mounted local configs save next selection without restarting, then start the exact saved revision", async () => {
  const dom = new JSDOM("<div id='root'></div>", {
    url: "https://desktop.test",
  });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const { createRoot } = await import("react-dom/client");
  const root = createRoot(document.getElementById("root"));
  let current = initial();
  const calls = [];
  let refuse = false;
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      calls.push({ command, args });
      assert.equal(args.owner, "owner");
      assert.equal(args.community, "wss://one.test");
      assert.equal(args.agent, "agent");
      if (command === "get_runtime_configurations")
        return structuredClone(current);
      if (command === "save_runtime_configurations") {
        assert.equal(args.expectedUpdatedAt, current.updatedAt);
        current = {
          ...current,
          configurations: args.configurations,
          updatedAt: "saved",
        };
        return structuredClone(current);
      }
      if (command === "start_runtime_configuration") {
        if (refuse) throw Error("missing prerequisite");
        current = { ...current, running: args.configuration };
        return {};
      }
      throw Error(command);
    },
  };
  const click = (text) =>
    React.act(async () => {
      const button = [...document.querySelectorAll("button")].find(
        (b) => b.textContent === text,
      );
      assert.ok(button, text);
      assert.equal(button.disabled, false, text);
      button.click();
    });
  try {
    await React.act(async () =>
      root.render(
        React.createElement(RuntimeConfigurationEditor, {
          scope: { owner: "owner", community: "wss://one.test" },
          agent: "agent",
          runtimes: [],
        }),
      ),
    );
    const picker = document.querySelector(
      'select[aria-label="Next runtime configuration"]',
    );
    assert.deepEqual(
      [...picker.options].map((o) => o.value),
      ["A", "B"],
    );
    await React.act(async () => {
      picker.value = "B";
      picker.dispatchEvent(new dom.window.Event("change", { bubbles: true }));
    });
    assert.equal(
      calls.filter((c) => c.command === "start_runtime_configuration").length,
      0,
    );
    assert.match(document.body.textContent, /Next Start: B/);
    assert.match(document.body.textContent, /Running revision: A \/ old/);
    refuse = true;
    await click("Start configuration");
    assert.match(
      document.body.textContent,
      /no new running configuration was confirmed/,
    );
    assert.match(document.body.textContent, /Running revision: A \/ old/);
    assert.deepEqual(calls.at(-1).args.configuration, reference("B"));
    refuse = false;
    await click("Start configuration");
    assert.match(document.body.textContent, /Running revision: B \/ old/);
    await click("Edit B");
    await click("Cancel configuration edit");
    assert.equal(
      calls.filter((c) => c.command === "save_runtime_configurations").length,
      1,
    );
  } finally {
    await React.act(async () => root.unmount());
    dom.window.close();
  }
});

test("late configuration read after unmount does not display another scope's settings", async () => {
  const dom = new JSDOM("<div id='root'></div>", {
    url: "https://desktop.test",
  });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const { createRoot } = await import("react-dom/client");
  const root = createRoot(document.getElementById("root"));
  let finish;
  window.__TAURI_INTERNALS__ = {
    invoke: () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  };
  await React.act(async () =>
    root.render(
      React.createElement(RuntimeConfigurationEditor, {
        scope: { owner: "owner", community: "wss://one.test" },
        agent: "agent",
        runtimes: [],
      }),
    ),
  );
  await React.act(async () => root.unmount());
  await React.act(async () => finish(initial()));
  assert.equal(document.body.textContent, "");
  dom.window.close();
});
