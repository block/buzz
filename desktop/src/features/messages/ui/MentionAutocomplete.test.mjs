import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
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

test("agent rows offer a one-way Always mention action", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const selected = [];
  const alwaysMentioned = [];
  const props = {
    suggestions: [suggestion],
    selectedIndex: 0,
    onSelect: (value) => selected.push(value),
    onAlwaysMentionAgent: (value) => alwaysMentioned.push(value),
    lockedAgentPubkeys: new Set(),
  };
  const view = render(React.createElement(MentionAutocomplete, props));

  assert.equal(
    view.queryByText("Hover an agent avatar to keep it addressed"),
    null,
  );
  fireEvent.click(
    view.getByRole("button", { name: "Always mention Agent Ada" }),
  );
  assert.deepEqual(alwaysMentioned, [suggestion]);
  assert.deepEqual(selected, []);

  view.rerender(
    React.createElement(MentionAutocomplete, {
      ...props,
      lockedAgentPubkeys: new Set(["agent-pubkey"]),
    }),
  );
  assert.ok(view.getByText("Always mentioned"));
  assert.equal(
    view.queryByRole("button", { name: "Always mention Agent Ada" }),
    null,
  );
});
