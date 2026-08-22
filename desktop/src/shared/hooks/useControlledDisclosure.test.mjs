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
    Element: dom.window.Element,
    Node: dom.window.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

/**
 * Drives a real controlled `<details>` through the hook, since the trap the
 * hook exists for lives in how `<details>` reports `toggle` — a unit test
 * against the returned callback alone could not observe it.
 */
async function renderDisclosure(initialPolicyOpen) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { useControlledDisclosure } = await import(
    "./useControlledDisclosure.ts"
  );

  function Probe({ policyOpen }) {
    const { onOpenChange, open } = useControlledDisclosure(policyOpen);
    return createElement(
      "details",
      {
        onToggle: (event) => onOpenChange(event.currentTarget.open),
        open,
      },
      createElement("summary", null, "header"),
      createElement("p", null, "body"),
    );
  }

  const view = render(createElement(Probe, { policyOpen: initialPolicyOpen }));
  const details = () => view.container.querySelector("details");
  return {
    details,
    // Re-run policy with a new answer, as streaming data does.
    setPolicy: (policyOpen) =>
      view.rerender(createElement(Probe, { policyOpen })),
    // What the reader does: the browser mutates open, THEN fires toggle.
    readerToggle: async () => {
      const { act, fireEvent } = await import("@testing-library/react");
      const element = details();
      await act(async () => {
        element.open = !element.open;
        fireEvent(element, new dom.window.Event("toggle"));
      });
    },
    // What a real browser additionally does after a PROGRAMMATIC open change:
    // echo a toggle that agrees with the state just rendered. jsdom does not
    // emit this, so it is injected — otherwise every guard test here would
    // pass vacuously.
    echoToggle: async () => {
      const { act, fireEvent } = await import("@testing-library/react");
      const element = details();
      await act(async () => {
        fireEvent(element, new dom.window.Event("toggle"));
      });
    },
  };
}

test("policy drives open state until the reader intervenes", async () => {
  const { details, setPolicy } = await renderDisclosure(true);
  assert.equal(details().open, true);

  setPolicy(false);
  assert.equal(details().open, false);

  setPolicy(true);
  assert.equal(details().open, true);
});

// The regression guard. Without it the echo is recorded as a reader choice and
// pins the row to its first policy answer, disabling every later transition.
test("a browser toggle echo of a policy-driven open is not a reader choice", async () => {
  const { details, echoToggle, setPolicy } = await renderDisclosure(true);
  assert.equal(details().open, true);

  await echoToggle();

  setPolicy(false);
  assert.equal(details().open, false);
});

test("the reader's choice outlives later policy changes", async () => {
  const { details, readerToggle, setPolicy } = await renderDisclosure(true);

  await readerToggle();
  assert.equal(details().open, false);

  // Policy still says open; the reader said otherwise and keeps winning.
  setPolicy(true);
  assert.equal(details().open, false);

  setPolicy(false);
  assert.equal(details().open, false);
});

test("the reader can also override policy in the opening direction", async () => {
  const { details, readerToggle, setPolicy } = await renderDisclosure(false);
  assert.equal(details().open, false);

  await readerToggle();
  assert.equal(details().open, true);

  setPolicy(false);
  assert.equal(details().open, true);
});

test("onOpenChange keeps a stable identity across renders", async () => {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { useControlledDisclosure } = await import(
    "./useControlledDisclosure.ts"
  );

  const seen = [];
  function Probe({ policyOpen }) {
    const { onOpenChange } = useControlledDisclosure(policyOpen);
    seen.push(onOpenChange);
    return null;
  }

  const view = render(createElement(Probe, { policyOpen: true }));
  view.rerender(createElement(Probe, { policyOpen: false }));

  assert.ok(seen.length >= 2);
  assert.equal(seen[0], seen[seen.length - 1]);
});
