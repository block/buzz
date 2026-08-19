import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  Object.assign(globalThis, {
    CustomEvent: dom.window.CustomEvent,
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    ResizeObserver: class {
      disconnect() {}
      observe() {}
      unobserve() {}
    },
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("agent rows offer an Always address pin", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const selected = [];
  const toggled = [];
  const props = {
    suggestions: [suggestion],
    selectedIndex: 0,
    onSelect: (value) => selected.push(value),
    onToggleAlwaysAddressAgent: (value) => toggled.push(value),
    lockedAgentPubkeys: new Set(),
  };
  const renderAutocomplete = (autocompleteProps) =>
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(MentionAutocomplete, autocompleteProps),
    );
  const view = render(renderAutocomplete(props));

  assert.equal(
    view.queryByText("Hover an agent avatar to keep it addressed"),
    null,
  );
  const action = view.getByRole("button", {
    name: "Always address Agent Ada",
  });
  assert.equal(action.getAttribute("aria-pressed"), "false");
  assert.equal(action.getAttribute("data-state"), "off");
  fireEvent.click(action);
  assert.deepEqual(toggled, [suggestion]);
  assert.deepEqual(selected, []);

  view.rerender(
    renderAutocomplete({
      ...props,
      lockedAgentPubkeys: new Set(["agent-pubkey"]),
    }),
  );
  const selectedAction = view.getByRole("button", {
    name: "Always address Agent Ada",
  });
  assert.equal(selectedAction.getAttribute("aria-pressed"), "true");
  assert.equal(selectedAction.getAttribute("data-state"), "on");
  fireEvent.click(selectedAction);
  assert.deepEqual(toggled, [suggestion, suggestion]);
});

test("Tab from the editor focuses the selected agent action", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const suggestions = [
    {
      pubkey: "agent-a",
      displayName: "Agent Ada",
      isAgent: true,
    },
    {
      pubkey: "agent-b",
      displayName: "Agent Bea",
      isAgent: true,
    },
  ];
  const view = render(
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(
        "form",
        null,
        React.createElement(
          "div",
          { "data-testid": "message-input-scroll" },
          React.createElement("input", { "aria-label": "Message" }),
        ),
        React.createElement(MentionAutocomplete, {
          suggestions,
          selectedIndex: 1,
          onSelect: () => {},
          onToggleAlwaysAddressAgent: () => {},
        }),
      ),
    ),
  );

  const input = view.getByRole("textbox", { name: "Message" });
  input.focus();
  fireEvent.keyDown(input, { key: "Tab" });

  assert.equal(
    document.activeElement,
    view.getByRole("button", { name: "Always address Agent Bea" }),
  );
});
