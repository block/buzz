import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    FocusEvent: dom.window.FocusEvent,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

async function renderHarness() {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { useComposerFocusOwnership } = await import(
    "./useComposerFocusOwnership.ts"
  );

  function Harness() {
    const formRef = React.useRef(null);
    const ownsFocus = useComposerFocusOwnership(formRef);
    return React.createElement(
      React.Fragment,
      null,
      React.createElement(
        "form",
        { "data-testid": "composer", ref: formRef },
        React.createElement("input", { "aria-label": "Editor" }),
        React.createElement("button", { type: "button" }, "Overlay control"),
        React.createElement("output", {
          "data-testid": "owned",
          "data-owned": String(ownsFocus),
        }),
      ),
      React.createElement("input", { "aria-label": "Elsewhere" }),
    );
  }

  const view = render(React.createElement(Harness));
  return {
    view,
    ownership: () => view.getByTestId("owned").getAttribute("data-owned"),
  };
}

test("tracks focus entering, moving within, and leaving the composer", async () => {
  const { act } = await import("react");
  const { view, ownership } = await renderHarness();

  assert.equal(ownership(), "false");

  const editor = view.getByRole("textbox", { name: "Editor" });
  await act(async () => editor.focus());
  assert.equal(ownership(), "true");

  // Focus handed from the editor to an overlay control stays owned — this is
  // the transition an editor-focus gate got wrong, unmounting the overlay
  // before the control it was handing focus to could receive it.
  const control = view.getByRole("button", { name: "Overlay control" });
  await act(async () => control.focus());
  assert.equal(ownership(), "true");

  const elsewhere = view.getByRole("textbox", { name: "Elsewhere" });
  await act(async () => elsewhere.focus());
  assert.equal(ownership(), "false");
});

test("an internal focus move never reports an unowned intermediate state", async () => {
  const React = await import("react");
  const { act } = React;
  const { render } = await import("@testing-library/react");
  const { useComposerFocusOwnership } = await import(
    "./useComposerFocusOwnership.ts"
  );

  const observed = [];
  function Harness() {
    const formRef = React.useRef(null);
    const ownsFocus = useComposerFocusOwnership(formRef);
    observed.push(ownsFocus);
    return React.createElement(
      "form",
      { ref: formRef },
      React.createElement("input", { "aria-label": "Editor" }),
      React.createElement("button", { type: "button" }, "Overlay control"),
    );
  }

  const view = render(React.createElement(Harness));
  const editor = view.getByRole("textbox", { name: "Editor" });
  const control = view.getByRole("button", { name: "Overlay control" });
  await act(async () => editor.focus());
  observed.length = 0;

  // relatedTarget mirrors a browser handing focus editor → overlay control.
  // The focusout handler must read it instead of assuming focus left.
  const { fireEvent } = await import("@testing-library/react");
  fireEvent.focusOut(editor, { relatedTarget: control });
  fireEvent.focusIn(control);

  assert.equal(observed.includes(false), false);
});
