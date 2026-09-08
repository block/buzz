import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";

async function withHookHarness(run) {
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  const originals = {
    window: globalThis.window,
    document: globalThis.document,
    navigator: Object.getOwnPropertyDescriptor(globalThis, "navigator"),
    MutationObserver: globalThis.MutationObserver,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    cancelAnimationFrame: globalThis.cancelAnimationFrame,
    act: globalThis.IS_REACT_ACT_ENVIRONMENT,
  };
  const frames = [];
  const requestAnimationFrame = (callback) => {
    frames.push(callback);
    return frames.length;
  };
  const cancelAnimationFrame = (id) => {
    frames[id - 1] = null;
  };
  Object.defineProperty(dom.window, "innerWidth", {
    configurable: true,
    value: 1000,
  });
  dom.window.requestAnimationFrame = requestAnimationFrame;
  dom.window.cancelAnimationFrame = cancelAnimationFrame;
  globalThis.window = dom.window;
  globalThis.document = dom.window.document;
  globalThis.MutationObserver = dom.window.MutationObserver;
  globalThis.requestAnimationFrame = requestAnimationFrame;
  globalThis.cancelAnimationFrame = cancelAnimationFrame;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;

  try {
    const React = (await import("react")).default;
    const { act } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const { useInlineTooltipPosition } = await import(
      "./useInlineTooltipPosition.ts"
    );
    let controls;
    function Probe() {
      controls = useInlineTooltipPosition();
      return null;
    }
    const root = createRoot(
      dom.window.document.body.appendChild(
        dom.window.document.createElement("div"),
      ),
    );
    await act(async () => root.render(React.createElement(Probe)));
    const flushFrames = async () => {
      const pending = frames.splice(0, frames.length).filter(Boolean);
      await act(async () => {
        for (const callback of pending) callback();
      });
    };
    const flushMutations = async () => {
      await act(async () => Promise.resolve());
    };

    await run({ act, controls, dom, flushFrames, flushMutations });
    await act(async () => root.unmount());
  } finally {
    globalThis.window = originals.window;
    globalThis.document = originals.document;
    globalThis.MutationObserver = originals.MutationObserver;
    globalThis.requestAnimationFrame = originals.requestAnimationFrame;
    globalThis.cancelAnimationFrame = originals.cancelAnimationFrame;
    if (originals.navigator) {
      Object.defineProperty(globalThis, "navigator", originals.navigator);
    } else {
      delete globalThis.navigator;
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = originals.act;
  }
}

function translateX(styleValue) {
  return (
    Number.parseFloat(styleValue.match(/translate(?:3d)?\(([-\d.]+)px/)?.[1]) ||
    0
  );
}

test("follows Radix wrapper placement without mutating the wrapper", async () => {
  await withHookHarness(
    async ({ controls, dom, flushFrames, flushMutations }) => {
      const wrapper = dom.window.document.createElement("div");
      const content = dom.window.document.createElement("div");
      wrapper.append(content);
      dom.window.document.body.append(wrapper);
      wrapper.style.transform = "translate(400px, 20px)";
      content.getBoundingClientRect = () => {
        const left =
          translateX(wrapper.style.transform) +
          (Number.parseFloat(wrapper.style.marginLeft) || 0) +
          (Number.parseFloat(content.style.translate) || 0);
        return {
          bottom: 50,
          height: 30,
          left,
          right: left + 100,
          top: 20,
          width: 100,
          x: left,
          y: 20,
          toJSON() {},
        };
      };
      const target = dom.window.document.createElement("span");
      target.getClientRects = () => [
        { bottom: 20, height: 20, left: 360, right: 430, top: 0, width: 70 },
        { bottom: 40, height: 20, left: 450, right: 600, top: 20, width: 150 },
      ];

      controls.contentRef(content);
      controls.onPointerMove({
        clientX: 395,
        clientY: 10,
        currentTarget: target,
      });
      await flushFrames();
      assert.equal(wrapper.style.marginLeft, "");
      assert.equal(content.getBoundingClientRect().left, 345);

      // Radix may place the popper wrapper after the pointer lifecycle. The
      // hook must follow that external placement while leaving its style owned
      // by Radix; mutating the wrapper is what made this ordering stale.
      wrapper.style.transform = "translate(480px, 20px)";
      await flushMutations();
      await flushFrames();

      assert.equal(wrapper.style.marginLeft, "");
      assert.equal(content.getBoundingClientRect().left, 345);
      assert.equal(content.getBoundingClientRect().right, 445);
    },
  );
});
