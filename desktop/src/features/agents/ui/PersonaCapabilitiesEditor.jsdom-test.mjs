import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { PersonaCapabilitiesEditor } from "./PersonaCapabilitiesEditor.tsx";
import { PersonaCapabilitiesImportNotice } from "./PersonaCapabilitiesImportNotice.tsx";
let root, container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

function renderEditor(value = {}) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  const state = { valid: true, changed: undefined };
  act(() =>
    root.render(
      React.createElement(PersonaCapabilitiesEditor, {
        value,
        onChange: (v) => {
          state.changed = v;
        },
        onValidityChange: (v) => {
          state.valid = v;
        },
      }),
    ),
  );
  return state;
}

function field(labelText) {
  const label = [...container.querySelectorAll("label")].find((l) =>
    l.textContent.startsWith(labelText),
  );
  assert.ok(label, `missing field ${labelText}`);
  return label.querySelector("input, textarea");
}

function type(input, text) {
  const proto =
    input.tagName === "TEXTAREA"
      ? window.HTMLTextAreaElement.prototype
      : window.HTMLInputElement.prototype;
  const setValue = Object.getOwnPropertyDescriptor(proto, "value").set;
  act(() => {
    setValue.call(input, text);
    input.dispatchEvent(new window.Event("input", { bubbles: true }));
  });
}

test("MCP editor keeps invalid draft visible and blocks submission until corrected", () => {
  const state = renderEditor();
  const input = field("MCP connections");
  type(input, "{");
  assert.equal(state.valid, false);
  assert.equal(input.value, "{");
  assert.equal(state.changed, undefined);
  assert.ok(container.querySelector('[role="alert"]'));
  type(
    input,
    '[{"name":"probe","command":"node","args":["two words"],"env":{"ROLE":"reviewer"}}]',
  );
  assert.equal(state.valid, true);
  assert.equal(state.changed.mcp_servers[0].args[0], "two words");
  assert.equal(state.changed.mcp_servers[0].env.ROLE, "reviewer");
});

test("MCP editor rejects fields Rust would silently drop and accepts a blank list", () => {
  const state = renderEditor();
  const input = field("MCP connections");
  for (const bad of [
    '[{"name":"probe","command":"node","arguments":["x"]}]',
    '[{"name":"probe","command":"node","args":"--x"}]',
    '[{"name":"probe","command":"node","env":{"TOKEN":7}}]',
  ]) {
    type(input, bad);
    assert.equal(state.valid, false, bad);
  }
  type(input, "");
  assert.equal(state.valid, true);
  assert.deepEqual(state.changed.mcp_servers, []);
});

test("skill folders keep a typed line break and trim pasted CRLF lines", () => {
  const state = renderEditor();
  const input = field("Skill folders");
  type(input, "skills/a\n");
  assert.equal(input.value, "skills/a\n");
  assert.deepEqual(state.changed.skills, ["skills/a"]);
  type(input, "skills/a\nskills/b");
  assert.deepEqual(state.changed.skills, ["skills/a", "skills/b"]);
  type(input, "skills/a\r\n  skills/My Skill \r\n");
  assert.deepEqual(state.changed.skills, ["skills/a", "skills/My Skill"]);
});

test("clearing every lifecycle script removes the hook assignment", () => {
  const state = renderEditor({ hooks: { on_start: "start.sh" } });
  type(field("Start script"), "");
  assert.equal(state.changed.hooks, undefined);
});

test("import notice shows executable settings without an expandable summary", () => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() =>
    root.render(
      React.createElement(PersonaCapabilitiesImportNotice, {
        value: { mcp_servers: [{ name: "probe", command: "node" }] },
      }),
    ),
  );
  assert.equal(container.querySelector("details"), null);
  const notice = container.querySelector(
    '[data-testid="persona-capabilities-import-notice"]',
  );
  assert.ok(notice.textContent.includes("may execute with your access"));
  assert.ok(notice.textContent.includes('"command": "node"'));
});
