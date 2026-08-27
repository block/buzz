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
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("SSH settings stay collapsed until explicitly expanded", async () => {
  const { createElement, useState } = await import("react");
  const { fireEvent, render, screen } = await import("@testing-library/react");
  const { CodexSshDisclosure } = await import("./CodexSshDisclosure.tsx");

  function Harness() {
    const [expanded, setExpanded] = useState(false);
    return createElement(
      CodexSshDisclosure,
      {
        connected: false,
        expanded,
        onExpandedChange: setExpanded,
      },
      createElement("p", null, "SSH fields"),
    );
  }

  render(createElement(Harness));
  const disclosure = screen.getByTestId("codex-ssh-disclosure");
  assert.equal(disclosure.getAttribute("aria-expanded"), "false");
  assert.equal(screen.queryByText("SSH fields"), null);

  fireEvent.click(disclosure);
  assert.equal(disclosure.getAttribute("aria-expanded"), "true");
  assert.ok(screen.getByText("SSH fields"));

  fireEvent.click(disclosure);
  assert.equal(disclosure.getAttribute("aria-expanded"), "false");
  assert.equal(screen.queryByText("SSH fields"), null);
});
