/**
 * Mounted regression test for #5366: a row being typed into must survive the
 * async runtime-catalog resolution.
 *
 * `reconcileRows` is unit-tested in `EnvVarsEditor.test.mjs`, but object
 * identity in the helper does not prove that the component's effect picks the
 * re-projection branch, nor that React preserves the focused DOM input across
 * the update. That is the part users feel: the catalog request is in flight for
 * seconds after a harness switch, and when it lands mid-typing the old code
 * rebuilt every row with a fresh `crypto.randomUUID()`, remounting the input
 * under the cursor so the rest of the keystrokes went nowhere.
 *
 * So this mounts the real component in jsdom, types a partial key, moves
 * `hiddenKeys` the way the resolved catalog does, and asserts the very same
 * input node is still focused and still accepts the rest of the key.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

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
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("a half-typed row keeps its focused input when the catalog resolves", async () => {
  const { createElement, useState } = await import("react");
  const { fireEvent, render, screen } = await import("@testing-library/react");
  const { EnvVarsEditor } = await import("./EnvVarsEditor.tsx");

  let latest = {};

  function Host({ hiddenKeys }) {
    const [value, setValue] = useState({});
    return createElement(EnvVarsEditor, {
      hiddenKeys,
      onChange: (next) => {
        latest = next;
        setValue(next);
      },
      value,
    });
  }

  // The catalog has not resolved yet, so nothing is hidden.
  const { rerender } = render(createElement(Host, { hiddenKeys: [] }));

  fireEvent.click(screen.getByTestId("env-vars-add"));
  const keyInput = screen.getByTestId("env-vars-key");
  keyInput.focus();
  fireEvent.change(keyInput, { target: { value: "BUZZ_AC" } });

  assert.ok(
    dom.window.document.activeElement === keyInput,
    "precondition: the row being typed into holds focus",
  );

  // The runtime catalog resolves and contributes a hidden key. `value` is
  // unchanged from what the editor last emitted — only `skipKeys` moves.
  rerender(createElement(Host, { hiddenKeys: ["BUZZ_RUNTIME_TOKEN"] }));

  // Identity, not `assert.equal`: a failed deep-equal on two jsdom nodes
  // spends ~45s building a diff nobody reads.
  assert.ok(
    screen.getByTestId("env-vars-key") === keyInput,
    "the key input must be the same DOM node, not a remounted one",
  );
  assert.ok(
    dom.window.document.activeElement === keyInput,
    "focus must survive the catalog update",
  );
  assert.equal(keyInput.value, "BUZZ_AC", "the partial key must survive");

  // Finish typing into that same input.
  fireEvent.change(keyInput, {
    target: { value: "BUZZ_ACP_PERMISSION_MODE" },
  });
  fireEvent.change(screen.getByTestId("env-vars-value"), {
    target: { value: "acceptEdits" },
  });

  assert.equal(
    latest.BUZZ_ACP_PERMISSION_MODE,
    "acceptEdits",
    "the whole key must reach the parent, not the truncated prefix",
  );
  assert.equal(
    latest.BUZZ_AC,
    undefined,
    "the truncated prefix must not be saved",
  );
});
