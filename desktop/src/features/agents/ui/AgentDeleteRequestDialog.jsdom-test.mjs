/**
 * The owner-confirmation surface for an agent-requested delete: one label
 * owner (the dialog title), explicit keyboard-operable confirmation, and a
 * visible error instead of a silent close.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { AgentDeleteRequestDialog } from "./AgentDeleteRequestDialog.tsx";

const persona = { id: "p1", displayName: "Scout" };

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

function mount(props) {
  const calls = { confirm: [], close: 0 };
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  const render = (extra = {}) =>
    act(() =>
      root.render(
        React.createElement(AgentDeleteRequestDialog, {
          error: null,
          instanceCount: 0,
          isPending: false,
          onClose: () => {
            calls.close += 1;
          },
          onConfirm: (p) => calls.confirm.push(p),
          ...props,
          ...extra,
        }),
      ),
    );
  render();
  return { calls, render };
}

const dialog = () => document.body.querySelector('[role="alertdialog"]');
const button = (name) =>
  [...document.body.querySelectorAll("button")].find(
    (b) => b.textContent === name,
  );
const textOf = (id) => document.getElementById(id)?.textContent;

test("confirmation names the agent and has one label owner", () => {
  mount({ target: { status: "ready", persona }, instanceCount: 2 });

  const el = dialog();
  assert.ok(el, "an alertdialog opens");
  assert.equal(textOf(el.getAttribute("aria-labelledby")), "Delete agent?");
  assert.match(
    textOf(el.getAttribute("aria-describedby")),
    /Delete Scout\. Also deletes 2 agent instances/,
  );
  assert.equal(el.getAttribute("aria-label"), null);
  for (const b of el.querySelectorAll("button")) {
    assert.equal(b.getAttribute("aria-label"), null);
  }
});

test("opens on Cancel so Enter cannot delete; Delete is a native button", () => {
  const { calls } = mount({ target: { status: "ready", persona } });

  assert.equal(document.activeElement, button("Cancel"));
  const del = button("Delete");
  assert.equal(del.tagName, "BUTTON");
  assert.equal(del.disabled, false);
  assert.deepEqual(calls.confirm, []);

  act(() => del.click());
  assert.deepEqual(calls.confirm, [persona]);
  assert.equal(calls.close, 0, "confirming must not close before the result");
});

test("Escape and Cancel dismiss without confirming", () => {
  const { calls } = mount({ target: { status: "ready", persona } });

  act(() => {
    document.dispatchEvent(
      new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
  });
  assert.equal(calls.close, 1);

  act(() => button("Cancel").click());
  assert.equal(calls.close, 2);
  assert.deepEqual(calls.confirm, []);
});

test("a failed delete stays visible in an alert and Delete is locked while pending", () => {
  const { render } = mount({ target: { status: "ready", persona } });

  render({ isPending: true });
  assert.equal(button("Delete").disabled, true);

  render({ error: "Relay refused the archive." });
  const alert = dialog().querySelector('[role="alert"]');
  assert.equal(alert?.textContent, "Relay refused the archive.");
  assert.equal(button("Delete").disabled, false);
});

test("an unknown agent shows an error dialog with one acknowledge action", () => {
  const { calls } = mount({
    target: { status: "error", message: "No personal agent is named “Ghost”." },
  });

  const el = dialog();
  assert.equal(
    textOf(el.getAttribute("aria-labelledby")),
    "Can't delete agent",
  );
  assert.match(
    textOf(el.getAttribute("aria-describedby")),
    /No personal agent is named “Ghost”/,
  );
  assert.equal(button("Delete"), undefined);
  act(() => button("OK").click());
  assert.equal(calls.close, 1);
  assert.deepEqual(calls.confirm, []);
});

test("renders nothing while the agent list loads", () => {
  mount({ target: { status: "loading" } });
  assert.equal(dialog(), null);
});
