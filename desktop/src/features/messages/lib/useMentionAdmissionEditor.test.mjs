import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  for (const key of [
    "window",
    "document",
    "DOMParser",
    "Element",
    "HTMLElement",
    "Node",
    "MutationObserver",
    "Event",
    "KeyboardEvent",
  ])
    globalThis[key] = dom.window[key];
  globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});
afterEach(async () => (await import("@testing-library/react")).cleanup());
after(() => dom.window.close());

async function tools() {
  return {
    React: await import("react"),
    ...(await import("@testing-library/react")),
    ...(await import("@tiptap/react")),
    StarterKit: (await import("@tiptap/starter-kit")).default,
    ...(await import("./useMentionAdmissionEditor.ts")),
  };
}

test("useEditor replacement can retire the captured editor before the admission effect", async () => {
  const {
    React,
    render,
    act,
    useEditor,
    EditorContent,
    StarterKit,
    useMentionAdmissionEditor,
  } = await tools();
  const instances = [];
  function Harness({ revision }) {
    const editor = useEditor({ extensions: [StarterKit] }, [revision]);
    if (editor && !instances.includes(editor)) instances.push(editor);
    const cancel = React.useCallback(() => {}, [revision]);
    useMentionAdmissionEditor(editor, cancel);
    return React.createElement(EditorContent, { editor });
  }
  const mounted = render(React.createElement(Harness, { revision: 0 }));
  await act(async () =>
    mounted.rerender(React.createElement(Harness, { revision: 1 })),
  );
  assert.equal(instances.length, 2);
  assert.equal(instances[0].isDestroyed, true);
  assert.equal(instances[1].isDestroyed, false);
});

test("real mount/unmount/remount and replacement retain guards without stale DOM listeners", async () => {
  const { React, render, Editor, StarterKit, useMentionAdmissionEditor } =
    await tools();
  const editor = new Editor({
    element: null,
    extensions: [StarterKit],
    content: "hello",
    // The chooser consumes these keys in production. Keep ProseMirror's
    // default Enter edit separate from the native cancellation assertion.
    editorProps: { handleKeyDown: () => true },
  });
  const replacement = new Editor({
    element: null,
    extensions: [StarterKit],
    editorProps: { handleKeyDown: () => true },
  });
  let cancellations = 0;
  const cancel = () => {
    cancellations++;
  };
  function Harness({ editor }) {
    useMentionAdmissionEditor(editor, cancel);
    return null;
  }
  const mounted = render(React.createElement(Harness, { editor }));
  function guarded(instance) {
    const element = instance.view.dom;
    for (const type of ["beforeinput", "pointerdown"]) {
      const before = cancellations;
      element.dispatchEvent(new Event(type));
      assert.equal(cancellations, before + 1, type);
    }
    for (const key of ["Enter", "Tab", " ", "ArrowLeft", "Escape", "x"]) {
      const before = cancellations;
      element.dispatchEvent(new KeyboardEvent("keydown", { key }));
      assert.equal(
        cancellations,
        before + (["Enter", "Tab", " "].includes(key) ? 0 : 1),
        key,
      );
    }
    let before = cancellations;
    instance.view.dispatch(instance.state.tr);
    assert.equal(cancellations, before, "no-op transaction");
    instance.commands.setTextSelection(1);
    assert.equal(cancellations, before + 1, "explicit selection transaction");
    before = cancellations;
    instance.commands.insertContent("a");
    assert.equal(cancellations, before + 1, "document transaction");
    return element;
  }
  function inert(element) {
    const before = cancellations;
    for (const type of ["beforeinput", "pointerdown"])
      element.dispatchEvent(new Event(type));
    element.dispatchEvent(new KeyboardEvent("keydown", { key: "x" }));
    assert.equal(
      cancellations,
      before,
      "retired DOM has no admission listeners",
    );
  }
  try {
    assert.throws(() => editor.view.dom, /editor view is not available/);
    editor.mount(document.createElement("div"));
    const first = guarded(editor);
    let before = cancellations;
    editor.unmount();
    assert.ok(cancellations > before, "unmount invalidates pending work");
    inert(first);
    editor.mount(document.createElement("div"));
    const second = guarded(editor);
    assert.notEqual(first, second);
    inert(first);
    before = cancellations;
    mounted.rerender(React.createElement(Harness, { editor: replacement }));
    assert.ok(cancellations > before, "replacement invalidates pending work");
    inert(second);
    replacement.mount(document.createElement("div"));
    const third = guarded(replacement);
    before = cancellations;
    replacement.destroy();
    assert.ok(cancellations > before, "destroy invalidates pending work");
    inert(third);
    assert.throws(() => replacement.view.dom, /editor view is not available/);
    mounted.unmount(); // cleanup must not read the now-unavailable view
  } finally {
    editor.destroy();
    replacement.destroy();
  }
});

test("unmount fences real pending admission across remount; mounted admission still commits", async () => {
  const { React, render, act, Editor, StarterKit, useMentionAdmissionEditor } =
    await tools();
  const { useMentionAdmission } = await import("./useMentionAdmission.ts");
  const editor = new Editor({ extensions: [StarterKit] });
  const scope = {};
  let admission;
  let commits = 0;
  function Harness() {
    admission = useMentionAdmission(scope);
    useMentionAdmissionEditor(editor, admission.cancel);
    return null;
  }
  const mounted = render(React.createElement(Harness));
  let release;
  const prepared = new Promise((resolve) => {
    release = resolve;
  });
  try {
    await act(async () =>
      admission.begin({
        key: {},
        valid: () => true,
        prepare: () => prepared,
        commit: () => {
          commits++;
        },
      }),
    );
    assert.equal(admission.status, "Checking access…");
    await act(async () => {
      editor.unmount();
      editor.mount(document.createElement("div"));
      release();
      await prepared;
    });
    assert.equal(commits, 0);
    assert.equal(admission.status, "");
    await act(async () =>
      admission.begin({
        key: {},
        valid: () => true,
        prepare: async () => {},
        commit: () => {
          commits++;
        },
      }),
    );
    assert.equal(commits, 1, "new mounted operation is not disabled");
  } finally {
    mounted.unmount();
    editor.destroy();
  }
});
