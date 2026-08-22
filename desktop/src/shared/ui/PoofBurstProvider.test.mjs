import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
let htmlAudioConstructions = 0;
const sources = [];

class FakeBufferSource {
  buffer = null;
  started = false;

  addEventListener() {}
  connect() {}
  start() {
    this.started = true;
  }
}

class FakeAudioContext {
  destination = {};
  state = "running";

  createBufferSource() {
    const source = new FakeBufferSource();
    sources.push(source);
    return source;
  }

  createGain() {
    return {
      connect() {},
      gain: { value: 1 },
    };
  }

  async decodeAudioData() {
    return { id: "poof-buffer" };
  }
}

before(() => {
  Object.assign(globalThis, {
    Audio: class {
      constructor() {
        htmlAudioConstructions += 1;
      }
    },
    AudioContext: FakeAudioContext,
    document: dom.window.document,
    Element: dom.window.Element,
    Image: dom.window.Image,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  globalThis.fetch = async () => ({
    arrayBuffer: async () => new ArrayBuffer(4),
    ok: true,
  });
});

after(() => dom.window.close());

test("poof effects never create resumable HTML media", async () => {
  const { createElement } = await import("react");
  const { act, fireEvent, render, waitFor } = await import(
    "@testing-library/react"
  );
  const { POOF_TRIGGER_CLASS, PoofBurstProvider } = await import(
    "./PoofBurstProvider.tsx"
  );

  const view = render(
    createElement(
      PoofBurstProvider,
      null,
      createElement("button", { className: POOF_TRIGGER_CLASS }, "Remove"),
    ),
  );

  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  assert.equal(htmlAudioConstructions, 0);

  fireEvent.click(view.getByRole("button", { name: "Remove" }));
  await waitFor(() => assert.equal(sources.at(-1)?.started, true));
  assert.equal(htmlAudioConstructions, 0);

  view.unmount();
});
