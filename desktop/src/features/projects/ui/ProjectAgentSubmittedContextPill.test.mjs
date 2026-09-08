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

test("keeps submitted context collapsed until explicitly expanded", async () => {
  const { createElement } = await import("react");
  const { fireEvent, render, screen } = await import("@testing-library/react");
  const { ProjectAgentSubmittedContextPill } = await import(
    "./ProjectAgentSubmittedContextPill.tsx"
  );
  const payload =
    '---\nCurrent Buzz project page:\n- Project: "Projects"\n- View: Activity';

  render(createElement(ProjectAgentSubmittedContextPill, { payload }));

  const trigger = screen.getByRole("button", { name: "Show sent context" });
  assert.equal(trigger.getAttribute("aria-expanded"), "false");
  assert.equal(
    screen.queryByTestId("project-agent-sent-context-payload"),
    null,
  );

  fireEvent.click(trigger);
  assert.equal(trigger.getAttribute("aria-expanded"), "true");
  const disclosed = screen.getByTestId("project-agent-sent-context-payload");
  assert.equal(disclosed.getAttribute("aria-label"), "Sent project context");
  assert.equal(disclosed.getAttribute("tabindex"), "0");
  assert.equal(disclosed.textContent, payload);

  fireEvent.click(trigger);
  assert.equal(
    screen.queryByTestId("project-agent-sent-context-payload"),
    null,
  );
});
