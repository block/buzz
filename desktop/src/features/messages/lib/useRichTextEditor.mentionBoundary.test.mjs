import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { TextSelection } from "@tiptap/pm/state";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

let act;
let cleanup;
let renderHook;
let useRichTextEditor;

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    DocumentFragment: dom.window.DocumentFragment,
    DOMParser: dom.window.DOMParser,
    Element: dom.window.Element,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    getSelection: dom.window.getSelection.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    InputEvent: dom.window.InputEvent,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
    window: dom.window,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  ({ act, cleanup, renderHook } = await import("@testing-library/react"));
  ({ useRichTextEditor } = await import("./useRichTextEditor.ts"));
});

afterEach(() => cleanup?.());
after(() => dom.window.close());

function createBeforeInput() {
  let prevented = false;
  return {
    data: "t",
    inputType: "insertText",
    isComposing: false,
    preventDefault() {
      prevented = true;
    },
    get prevented() {
      return prevented;
    },
  };
}

function setMentionContent(editor, name = "Reinhold") {
  editor.commands.setContent({
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [{ type: "text", text: `@${name} ` }],
      },
    ],
  });
  const end = 1 + editor.state.doc.textContent.length;
  editor.view.dispatch(
    editor.state.tr.setSelection(TextSelection.create(editor.state.doc, end)),
  );
}

function invokeProductionBeforeInput(editor, event) {
  const handler = editor.view.props.handleDOMEvents?.beforeinput;
  assert.equal(typeof handler, "function");
  return handler(editor.view, event);
}

test("the mounted production beforeinput callback dispatches only at the decorated boundary and uses current agent names", async () => {
  const hook = renderHook(
    ({ agentMentionNames }) => useRichTextEditor({ agentMentionNames }),
    { initialProps: { agentMentionNames: ["Reinhold"] } },
  );

  assert.ok(hook.result.current.editor);
  const editor = hook.result.current.editor;
  act(() => setMentionContent(editor));

  let dispatchCount = 0;
  const originalDispatch = editor.view.dispatch.bind(editor.view);
  editor.view.dispatch = (transaction) => {
    dispatchCount += 1;
    originalDispatch(transaction);
  };

  const decoratedEvent = createBeforeInput();
  let handled;
  act(() => {
    handled = invokeProductionBeforeInput(editor, decoratedEvent);
  });

  assert.equal(handled, true);
  assert.equal(decoratedEvent.prevented, true);
  assert.equal(dispatchCount, 1);
  assert.equal(editor.state.doc.textContent, "@Reinhold t");
  assert.equal(
    editor.state.doc.textContent.codePointAt("@Reinhold".length),
    0x20,
  );
  assert.equal(editor.state.selection.from, 1 + "@Reinhold t".length);

  act(() => {
    hook.rerender({ agentMentionNames: ["Fizz"] });
  });
  act(() => setMentionContent(editor));
  dispatchCount = 0;

  const undecoratedEvent = createBeforeInput();
  act(() => {
    handled = invokeProductionBeforeInput(editor, undecoratedEvent);
  });

  assert.equal(handled, false);
  assert.equal(undecoratedEvent.prevented, false);
  assert.equal(dispatchCount, 0);
  assert.equal(editor.state.doc.textContent, "@Reinhold ");

  act(() => setMentionContent(editor, "Fizz"));
  dispatchCount = 0;
  const refreshedEvent = createBeforeInput();
  act(() => {
    handled = invokeProductionBeforeInput(editor, refreshedEvent);
  });

  assert.equal(handled, true);
  assert.equal(refreshedEvent.prevented, true);
  assert.equal(dispatchCount, 1);
  assert.equal(editor.state.doc.textContent, "@Fizz t");
});
