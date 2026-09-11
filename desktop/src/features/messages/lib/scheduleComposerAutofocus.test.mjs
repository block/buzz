import assert from "node:assert/strict";
import test from "node:test";
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { JSDOM } from "jsdom";
import { scheduleComposerAutofocus } from "./scheduleComposerAutofocus.ts";

for (const scenario of [
  "initial",
  "pointer",
  "keyboard",
  "focus",
  "retired",
  "destroyed",
  "disabled",
  "navigation",
  "explicit",
  "iOS",
  "Android",
  "Safari",
]) {
  test(`automatic focus commit: ${scenario}`, () => {
    const dom = new JSDOM(
      '<body><button>menu</button><div id="editor"></div></body>',
      { pretendToBeVisual: true },
    );
    const saved = new Map();
    for (const key of [
      "window",
      "document",
      "navigator",
      "HTMLElement",
      "Node",
      "getComputedStyle",
      "requestAnimationFrame",
      "cancelAnimationFrame",
    ]) {
      saved.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
      Object.defineProperty(globalThis, key, {
        configurable: true,
        writable: true,
        value:
          typeof dom.window[key] === "function" &&
          key.includes("AnimationFrame")
            ? dom.window[key].bind(dom.window)
            : dom.window[key],
      });
    }
    const editor = new Editor({
      element: document.querySelector("#editor"),
      extensions: [StarterKit],
      editorProps: { handleScrollToSelection: () => true },
      content: "<p>hello</p>",
    });
    const held = [];
    window.requestAnimationFrame = (callback) => {
      held.push(callback);
      return held.length;
    };
    window.cancelAnimationFrame = () => {}; // A callback already delivered to a scheduler can still arrive.
    globalThis.requestAnimationFrame = window.requestAnimationFrame;
    const editorDOM = editor.view.dom;
    const mobile = scenario === "iOS" || scenario === "Android";
    const safari = scenario === "Safari";
    if (mobile)
      Object.defineProperty(navigator, "platform", {
        value: scenario === "iOS" ? "iPhone" : "Android",
      });
    if (safari)
      Object.defineProperty(navigator, "userAgent", {
        value: "Version/18.0 Safari/605.1.15",
      });
    const nativeFocus = editorDOM.focus.bind(editorDOM);
    const focusArgs = [];
    editorDOM.focus = (...args) => {
      focusArgs.push(args);
      nativeFocus(...args);
    };
    let disabled = false;
    const cancel = scheduleComposerAutofocus(editor, () => disabled);
    try {
      assert.equal(held.length, 1);
      if (mobile || safari) {
        assert.equal(
          document.activeElement,
          editorDOM,
          "platform preparation is immediate",
        );
        assert.deepEqual(focusArgs[0], safari ? [{ preventScroll: true }] : []);
        document.querySelector("button").focus();
      }
      if (scenario === "pointer")
        document.body.dispatchEvent(
          new window.Event("pointerdown", { bubbles: true }),
        );
      if (scenario === "keyboard")
        document.body.dispatchEvent(
          new window.Event("keydown", { bubbles: true }),
        );
      if (scenario === "focus" || scenario === "explicit")
        document.querySelector("button").focus();
      if (scenario === "retired" || scenario === "navigation") cancel();
      if (scenario === "destroyed") editor.destroy();
      if (scenario === "disabled") disabled = true;
      if (scenario === "navigation")
        scheduleComposerAutofocus(editor, () => disabled);
      for (const callback of held) callback(0);
      assert.equal(
        document.activeElement === editorDOM,
        ["initial", "navigation"].includes(scenario),
      );
      if (scenario === "initial" || scenario === "navigation")
        assert.equal(editor.state.selection.from, 6);
      if (scenario === "explicit") {
        editor.commands.focus("end");
        held.at(-1)(0);
        assert.equal(document.activeElement, editor.view.dom);
      }
    } finally {
      cancel();
      if (!editor.isDestroyed) editor.destroy();
      dom.window.close();
      for (const [key, descriptor] of saved) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor);
        else delete globalThis[key];
      }
    }
  });
}
