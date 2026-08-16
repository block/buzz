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

test("empty projects state renders a create-project action that fires the provided callback", async () => {
  const { createElement } = await import("react");
  const { fireEvent, render, screen } = await import("@testing-library/react");
  const { EmptyState } = await import("./ProjectCards.tsx");

  let createClicked = false;
  render(
    createElement(EmptyState, {
      onCreateProject: () => {
        createClicked = true;
      },
    }),
  );

  assert.ok(screen.getByText("No projects yet"));
  const createButton = screen.getByRole("button", { name: "Create project" });
  fireEvent.click(createButton);
  assert.equal(createClicked, true);
});
