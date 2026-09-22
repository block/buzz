import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";
import * as React from "react";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const frames = new Map();
let frameId = 0;
const editors = [];
before(() =>
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Element: dom.window.Element,
    Node: dom.window.Node,
    MutationObserver: dom.window.MutationObserver,
    getComputedStyle: dom.window.getComputedStyle,
    requestAnimationFrame: (callback) => {
      frames.set(++frameId, callback);
      return frameId;
    },
    cancelAnimationFrame: (id) => frames.delete(id),
    IS_REACT_ACT_ENVIRONMENT: true,
  }),
);
afterEach(async () => {
  (await import("@testing-library/react")).cleanup();
  for (const editor of editors.splice(0)) editor.destroy();
  frames.clear();
  document.body.replaceChildren();
});
after(() => dom.window.close());

for (const modality of ["pointer", "keyboard"]) {
  test(`toolbar ${modality} dispatch synchronizes the list caret before deferred focus`, async () => {
    const { Editor } = await import("@tiptap/core");
    const { default: StarterKit } = await import("@tiptap/starter-kit");
    const { FormattingToolbar } = await import("./FormattingToolbar.tsx");
    const { TooltipProvider } = await import("@/shared/ui/tooltip");
    const { render, fireEvent, act } = await import("@testing-library/react");
    const element = document.createElement("div");
    document.body.append(element);
    const editor = new Editor({
      element,
      extensions: [StarterKit.configure({ trailingNode: false })],
      content: "<p>before<br></p>",
    });
    editors.push(editor);
    editor.commands.setTextSelection(8);
    editor.view.focus();
    const toolbar = render(
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(FormattingToolbar, { editor }),
      ),
    );
    const button = toolbar.getByRole("button", {
      name: "Bullet list",
      exact: true,
    });
    act(() => {
      if (modality === "pointer") fireEvent.mouseDown(button);
      // Model the native button focus transition; jsdom does not perform it.
      button.focus();
      fireEvent.click(button);
    });
    assert.equal(
      editor.view.hasFocus(),
      true,
      "format dispatch must not wait for Tiptap's frame",
    );
    assert.equal(editor.state.selection.$from.parent.type.name, "paragraph");
    assert.equal(editor.state.selection.$from.node(-1).type.name, "listItem");
    const selection = document.getSelection();
    assert.ok(
      editor.view.dom.querySelector("li").contains(selection.anchorNode),
      "mapped list caret must reach DOM before ordinary typing",
    );
    assert.equal(editor.view.dom.querySelector("p").textContent, "before");
  });
}
