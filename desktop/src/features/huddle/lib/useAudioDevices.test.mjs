import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

// jsdom does not implement `navigator.mediaDevices`, so the default environment
// here IS the non-secure-context case from #3118: the property is simply
// absent. Mounting a component that touches it unguarded throws during the
// mount effect and takes the tree down.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  // `globalThis.navigator` is getter-only on Node 24, so Object.assign cannot
  // reach it. Neither Node's nor jsdom's navigator implements `mediaDevices`,
  // which is exactly the state under test.
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("mounts without throwing when navigator.mediaDevices is absent", async () => {
  assert.equal(
    globalThis.navigator.mediaDevices,
    undefined,
    "precondition: jsdom exposes no mediaDevices",
  );

  const { createElement, useRef } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { useAudioDevices } = await import("./useAudioDevices.ts");

  function Harness() {
    const workletRef = useRef(null);
    const { audioDevices } = useAudioDevices(workletRef);
    return createElement("p", null, `devices:${audioDevices.length}`);
  }

  render(createElement(Harness));

  // Rendered at all means the mount effect did not throw; empty list means it
  // degraded rather than inventing devices.
  assert.ok(screen.getByText("devices:0"));
});
