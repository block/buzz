import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { showMentionAgentProvenanceMarker } from "./MentionAutocomplete.tsx";

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

test("agent rows offer automatic mention controls", async () => {
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
  const rowAction = view.getByRole("button", {
    name: "Mention Agent Ada",
  });
  fireEvent.mouseDown(rowAction);
  assert.deepEqual(selected, [suggestion]);

  const action = view.getByRole("button", {
    name: "Automatically mention Agent Ada",
  });
  assert.equal(action.getAttribute("aria-pressed"), "false");
  assert.equal(action.getAttribute("data-state"), "off");
  const inactivePin = action.querySelector(
    '[data-testid="mention-auto-pin-icon"]',
  );
  assert.match(inactivePin?.getAttribute("class") ?? "", /\blucide-pin\b/);
  assert.equal(inactivePin?.getAttribute("fill"), "none");
  fireEvent.click(action);
  assert.deepEqual(toggled, [suggestion]);
  assert.deepEqual(selected, [suggestion]);

  view.rerender(
    renderAutocomplete({
      ...props,
      lockedAgentPubkeys: new Set(["agent-pubkey"]),
    }),
  );
  const selectedAction = view.getByRole("button", {
    name: "Don't automatically mention Agent Ada in this thread",
  });
  assert.equal(selectedAction.getAttribute("aria-pressed"), "true");
  assert.equal(selectedAction.getAttribute("data-state"), "on");
  const activePin = selectedAction.querySelector(
    '[data-testid="mention-auto-pin-icon"]',
  );
  assert.equal(activePin?.getAttribute("fill"), "currentColor");
  fireEvent.click(selectedAction);
  assert.deepEqual(toggled, [suggestion, suggestion]);
});

test("automatic mention setting is visible by default without an options ingress", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const changes = [];
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const view = render(
    React.createElement(MentionAutocomplete, {
      suggestions: [suggestion],
      selectedIndex: 0,
      onSelect: () => {},
      keepMentionedAgentsPinned: true,
      onKeepMentionedAgentsPinnedChange: (value) => changes.push(value),
    }),
  );

  assert.equal(view.queryByRole("button", { name: "Options" }), null);
  assert.ok(view.getByTestId("mention-options-settings"));
  const settings = view.getByTestId("mention-options-settings");
  assert.ok(settings.classList.contains("w-80"));
  assert.ok(settings.classList.contains("max-w-full"));
  assert.ok(settings.parentElement?.classList.contains("justify-end"));
  const toggle = view.getByRole("switch", {
    name: "Automatically mention agents",
  });
  assert.equal(toggle.getAttribute("data-state"), "checked");
  assert.ok(view.getByText("Address selected agents in thread replies"));
  assert.ok(view.getByRole("button", { name: "Mention Agent Ada" }));

  fireEvent.click(toggle);
  assert.deepEqual(changes, [false]);
});

test("automatic selection updates the visible setting in place", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const props = {
    suggestions: [suggestion],
    selectedIndex: 0,
    onSelect: () => {},
    keepMentionedAgentsPinned: false,
    onKeepMentionedAgentsPinnedChange: () => {},
  };
  const view = render(React.createElement(MentionAutocomplete, props));
  const settings = view.getByTestId("mention-options-settings");
  const toggle = view.getByRole("switch", {
    name: "Automatically mention agents",
  });
  assert.equal(toggle.getAttribute("data-state"), "unchecked");

  view.rerender(
    React.createElement(MentionAutocomplete, {
      ...props,
      keepMentionedAgentsPinned: true,
      openOptionsRequest: 1,
    }),
  );
  assert.equal(view.getByTestId("mention-options-settings"), settings);
  assert.equal(toggle.getAttribute("data-state"), "checked");
});

test("clicking outside dismisses the tray without intercepting its trigger", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  let dismissCount = 0;
  const view = render(
    React.createElement(
      React.Fragment,
      null,
      React.createElement(
        "form",
        null,
        React.createElement(
          "button",
          { "data-mention-picker-trigger": "", type: "button" },
          "@",
        ),
        React.createElement(MentionAutocomplete, {
          suggestions: [suggestion],
          selectedIndex: 0,
          onDismiss: () => {
            dismissCount += 1;
          },
          onSelect: () => {},
        }),
      ),
      React.createElement("button", { type: "button" }, "Outside"),
      React.createElement(
        "form",
        null,
        React.createElement(
          "button",
          { "data-mention-picker-trigger": "", type: "button" },
          "Other @",
        ),
      ),
    ),
  );

  fireEvent.pointerDown(
    view.getByRole("button", { name: "Mention Agent Ada" }),
  );
  assert.equal(dismissCount, 0);

  fireEvent.pointerDown(view.getByRole("button", { name: "@" }));
  assert.equal(dismissCount, 0);

  fireEvent.pointerDown(view.getByTestId("mention-autocomplete-layer"));
  assert.equal(dismissCount, 1);

  fireEvent.pointerDown(view.getByRole("button", { name: "Outside" }));
  assert.equal(dismissCount, 2);

  fireEvent.pointerDown(view.getByRole("button", { name: "Other @" }));
  assert.equal(dismissCount, 3);
});

test("collision npubs sit inline with agent metadata", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const suggestions = [
    {
      pubkey: "a".repeat(64),
      displayName: "Same Name",
      isAgent: true,
      ownerLabel: "you",
    },
    {
      pubkey: "b".repeat(64),
      displayName: "Same Name",
      isAgent: true,
      ownerLabel: "you",
    },
  ];
  const view = render(
    React.createElement(MentionAutocomplete, {
      suggestions,
      selectedIndex: 0,
      onSelect: () => {},
    }),
  );

  const agentIcons = view.getAllByTestId("mention-agent-icon");
  const collisionNpubs = view.getAllByTestId("mention-collision-npub");
  assert.equal(collisionNpubs.length, 2);
  for (const [index, npub] of collisionNpubs.entries()) {
    const agentMetadata = agentIcons[index].closest("span")?.parentElement;
    assert.equal(npub.parentElement, agentMetadata);
    assert.match(agentMetadata?.textContent ?? "", /agentmanaged by younpub1/);
    assert.match(npub.className, /(?:^|\s)-translate-y-0\.5(?:\s|$)/);
    assert.match(npub.className, /(?:^|\s)leading-none(?:\s|$)/);
    assert.match(agentMetadata?.className ?? "", /(?:^|\s)min-h-3\.5(?:\s|$)/);
  }
});

test("does not intercept Tab from the editor", async () => {
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
  const wasNotCancelled = fireEvent.keyDown(input, { key: "Tab" });

  assert.equal(wasNotCancelled, true);
  assert.equal(document.activeElement, input);
});
function suggestion(agentProvenance) {
  return {
    pubkey: "1".repeat(64),
    displayName: "Carl",
    isAgent: true,
    agentProvenance,
  };
}

test("duplicate owned agents mark only the other setup", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-here")),
    false,
  );
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-elsewhere")),
    true,
  );
});

test("unique agents mark the same not-locally-managed provenance", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-elsewhere")),
    true,
  );
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-here")),
    false,
  );
});

test("agents without trustworthy provenance omit management provenance", () => {
  assert.equal(showMentionAgentProvenanceMarker(suggestion(undefined)), false);
});

for (const duplicate of [false, true]) {
  test(`rendered provenance is truthful (${duplicate ? "duplicate" : "unique"} names)`, async () => {
    const React = await import("react");
    const { render } = await import("@testing-library/react");
    const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
    const { TooltipProvider } = await import("@/shared/ui/tooltip");
    const remote = suggestion("managed-elsewhere");
    const suggestions = duplicate
      ? [remote, { ...suggestion("managed-here"), pubkey: "2".repeat(64) }]
      : [remote];
    const view = render(
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(MentionAutocomplete, {
          suggestions,
          selectedIndex: 0,
          onSelect() {},
        }),
      ),
    );
    const markers = view.getAllByRole("img", {
      name: "Not managed on this device",
    });
    assert.equal(markers.length, 1);
    assert.equal(markers[0].title, "Not managed on this device");
    assert.equal(view.queryByTitle("From another Buzz setup"), null);
  });
}
