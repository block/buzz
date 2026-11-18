/**
 * The create (`PersonaModelCombobox`) and edit (`PersonaDropdownField`) model
 * menus show a harness-provided description as a second line, fed through the
 * real option builder each dialog uses.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { PersonaDropdownField } from "./PersonaDropdownField.tsx";
import { PersonaModelCombobox } from "./PersonaModelCombobox.tsx";
import { modelDropdownOptions } from "./relayMeshModelPicker.ts";

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
globalThis.window.HTMLElement.prototype.scrollIntoView ??= () => {};
globalThis.window.HTMLElement.prototype.hasPointerCapture ??= () => false;
globalThis.window.HTMLElement.prototype.releasePointerCapture ??= () => {};

const models = [
  { id: "haiku", label: "Haiku", description: "Haiku 4.5 · Fastest" },
  { id: "claude-sonnet-4-6", label: "Sonnet 4.6" },
];

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

function mount(element) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => root.render(element));
}

function openMenu() {
  const trigger = container.querySelector("button");
  act(() => {
    trigger.dispatchEvent(
      new window.PointerEvent("pointerdown", { bubbles: true, button: 0 }),
    );
    trigger.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  });
}

function assertDescriptions() {
  const text = document.body.textContent;
  assert.match(text, /Haiku 4\.5 · Fastest/);
  assert.match(text, /Sonnet 4\.6/);
  assert.equal(
    [...document.body.querySelectorAll("span")].filter(
      (el) => el.textContent === "Haiku 4.5 · Fastest",
    ).length,
    1,
  );
}

const menuProps = (options) => ({
  id: "model",
  onValueChange: () => {},
  options,
  placeholder: "Default model",
  value: "haiku",
});

test("create menu shows the model description as a second line", () => {
  const options = modelDropdownOptions({
    options: models,
    loading: false,
    loadingValue: "__loading__",
    allowCustom: true,
  });
  mount(React.createElement(PersonaModelCombobox, menuProps(options)));
  openMenu();
  assertDescriptions();
});

for (const globalModel of ["", "claude-opus-5"]) {
  test(`edit menu shows the model description with globalModel ${JSON.stringify(globalModel)}`, () => {
    const options = modelDropdownOptions({
      options: models,
      loading: false,
      loadingValue: "__loading__",
      allowCustom: true,
      globalModel,
    });
    mount(React.createElement(PersonaDropdownField, menuProps(options)));
    openMenu();
    assertDescriptions();
  });
}

// Edit passes the inherited global model and its label; the discovered
// default row keeps its own label unless a global model actually overrides it.
for (const [globalModel, globalModelLabel, expected] of [
  ["", "Default model", "Default model (Opus)"],
  [
    "claude-sonnet-5",
    "Default model (claude-sonnet-5)",
    "Default model (claude-sonnet-5)",
  ],
]) {
  test(`edit menu default row reads ${JSON.stringify(expected)}`, () => {
    const options = modelDropdownOptions({
      options: [{ id: "", label: "Default model (Opus)" }, ...models],
      loading: false,
      loadingValue: "__loading__",
      allowCustom: true,
      globalModel,
      globalModelLabel,
    });
    mount(
      React.createElement(PersonaDropdownField, {
        ...menuProps(options),
        value: options[0].value,
      }),
    );
    openMenu();
    const texts = [...document.body.querySelectorAll("*")].map(
      (el) => el.textContent,
    );
    assert.ok(texts.includes(expected));
    assert.ok(!texts.includes("Default model"));
  });
}
