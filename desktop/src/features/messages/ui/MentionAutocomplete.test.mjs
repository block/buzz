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
    composerOwnsFocus: true,
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
    name: "Don't automatically mention Agent Ada in this conversation",
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

test("options expand in place without replacing the people list", async () => {
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
      composerOwnsFocus: true,
      onSelect: () => {},
      keepMentionedAgentsPinned: true,
      onKeepMentionedAgentsPinnedChange: (value) => changes.push(value),
    }),
  );

  const options = view.getByRole("button", { name: "Options" });
  assert.equal(options.getAttribute("aria-expanded"), "false");
  assert.match(options.parentElement?.className ?? "", /(?:^|\s)w-24(?:\s|$)/);
  assert.ok(view.getByRole("button", { name: "Mention Agent Ada" }));
  assert.equal(
    view.queryByRole("switch", { name: "Automatically mention agents" }),
    null,
  );

  fireEvent.click(options);
  assert.equal(options.getAttribute("aria-expanded"), "true");
  const toggle = view.getByRole("switch", {
    name: "Automatically mention agents",
  });
  assert.equal(toggle.getAttribute("data-state"), "checked");
  assert.ok(view.getByText("After you mention them once"));
  assert.ok(view.getByRole("button", { name: "Mention Agent Ada" }));

  fireEvent.click(toggle);
  assert.deepEqual(changes, [false]);

  view.rerender(
    React.createElement(MentionAutocomplete, {
      suggestions: [],
      selectedIndex: 0,
      composerOwnsFocus: true,
      onSelect: () => {},
      keepMentionedAgentsPinned: false,
      onKeepMentionedAgentsPinnedChange: (value) => changes.push(value),
    }),
  );
  assert.equal(view.queryByRole("button", { name: "Options" }), null);

  view.rerender(
    React.createElement(MentionAutocomplete, {
      suggestions: [suggestion],
      selectedIndex: 0,
      composerOwnsFocus: true,
      onSelect: () => {},
      keepMentionedAgentsPinned: false,
      onKeepMentionedAgentsPinnedChange: (value) => changes.push(value),
    }),
  );
  assert.equal(
    view.getByRole("button", { name: "Options" }).getAttribute("aria-expanded"),
    "false",
  );
  assert.equal(
    view.queryByRole("switch", { name: "Automatically mention agents" }),
    null,
  );
  assert.ok(view.getByRole("button", { name: "Mention Agent Ada" }));
});

test("automatic selection loads the setting once, then updates it in place", async () => {
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
    composerOwnsFocus: true,
    onSelect: () => {},
    keepMentionedAgentsPinned: false,
    onKeepMentionedAgentsPinnedChange: () => {},
  };
  const view = render(
    React.createElement(MentionAutocomplete, {
      ...props,
      openOptionsRequest: 0,
    }),
  );

  view.rerender(
    React.createElement(MentionAutocomplete, {
      ...props,
      openOptionsRequest: 1,
    }),
  );
  assert.equal(
    view.getByRole("button", { name: "Options" }).getAttribute("aria-expanded"),
    "true",
  );
  const toggle = view.getByRole("switch", {
    name: "Automatically mention agents",
  });
  const settings = view.getByTestId("mention-options-settings");
  assert.equal(toggle.getAttribute("data-state"), "unchecked");

  view.rerender(
    React.createElement(MentionAutocomplete, {
      ...props,
      keepMentionedAgentsPinned: true,
      openOptionsRequest: 2,
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
          composerOwnsFocus: true,
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
      composerOwnsFocus: true,
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

test("focusMentionOptionsTrigger hands focus to the Options trigger", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { MentionAutocomplete, focusMentionOptionsTrigger } = await import(
    "./MentionAutocomplete.tsx"
  );
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const view = render(
    React.createElement(
      "form",
      { "data-testid": "composer-form" },
      React.createElement("input", { "aria-label": "Message" }),
      React.createElement(MentionAutocomplete, {
        suggestions: [suggestion],
        selectedIndex: 0,
        composerOwnsFocus: true,
        onSelect: () => {},
        keepMentionedAgentsPinned: true,
        onKeepMentionedAgentsPinnedChange: () => {},
      }),
    ),
  );

  const form = view.getByTestId("composer-form");
  const input = view.getByRole("textbox", { name: "Message" });
  input.focus();

  assert.equal(focusMentionOptionsTrigger(form), true);
  assert.equal(
    document.activeElement,
    view.getByTestId("mention-options-trigger"),
  );
});

test("focusMentionOptionsTrigger declines when no Options surface renders", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { MentionAutocomplete, focusMentionOptionsTrigger } = await import(
    "./MentionAutocomplete.tsx"
  );
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const view = render(
    React.createElement(
      "form",
      { "data-testid": "composer-form" },
      React.createElement("input", { "aria-label": "Message" }),
      // No onKeepMentionedAgentsPinnedChange: composers without audience
      // controls render no Options surface, so the key event must fall
      // through to its default backward focus move instead of stranding
      // focus.
      React.createElement(MentionAutocomplete, {
        suggestions: [suggestion],
        selectedIndex: 0,
        composerOwnsFocus: true,
        onSelect: () => {},
      }),
    ),
  );

  const form = view.getByTestId("composer-form");
  const input = view.getByRole("textbox", { name: "Message" });
  input.focus();

  assert.equal(focusMentionOptionsTrigger(form), false);
  assert.equal(document.activeElement, input);
  assert.equal(focusMentionOptionsTrigger(null), false);
});

test("Escape inside the overlay returns focus to the editor and dismisses", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete, focusMentionOptionsTrigger } = await import(
    "./MentionAutocomplete.tsx"
  );
  const dismissals = [];
  const view = render(
    React.createElement(
      "form",
      { "data-testid": "composer-form" },
      React.createElement("input", {
        "aria-label": "Message",
        "data-testid": "message-input",
      }),
      React.createElement(MentionAutocomplete, {
        suggestions: [
          {
            pubkey: "agent-pubkey",
            displayName: "Agent Ada",
            isAgent: true,
          },
        ],
        selectedIndex: 0,
        composerOwnsFocus: true,
        onDismiss: () => dismissals.push(true),
        onSelect: () => {},
        keepMentionedAgentsPinned: true,
        onKeepMentionedAgentsPinnedChange: () => {},
      }),
    ),
  );

  const form = view.getByTestId("composer-form");
  assert.equal(focusMentionOptionsTrigger(form), true);
  const trigger = view.getByTestId("mention-options-trigger");
  const wasNotCancelled = fireEvent.keyDown(trigger, { key: "Escape" });

  assert.equal(wasNotCancelled, false);
  assert.equal(document.activeElement, view.getByTestId("message-input"));
  assert.deepEqual(dismissals, [true]);
});

test("renders nothing while the composer does not own focus", async () => {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const props = {
    suggestions: [
      {
        pubkey: "agent-pubkey",
        displayName: "Agent Ada",
        isAgent: true,
      },
    ],
    selectedIndex: 0,
    composerOwnsFocus: false,
    onSelect: () => {},
  };
  const view = render(React.createElement(MentionAutocomplete, props));

  assert.equal(view.queryByTestId("mention-autocomplete-layer"), null);

  view.rerender(
    React.createElement(MentionAutocomplete, {
      ...props,
      composerOwnsFocus: true,
    }),
  );
  assert.ok(view.getByTestId("mention-autocomplete-layer"));
});

test("container presses do not blur the editor out from under the overlay", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MentionAutocomplete } = await import("./MentionAutocomplete.tsx");
  const mention = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const selected = [];
  const pinnedChanges = [];
  const view = render(
    React.createElement(MentionAutocomplete, {
      suggestions: [mention],
      selectedIndex: 0,
      composerOwnsFocus: true,
      onSelect: (value) => selected.push(value),
      keepMentionedAgentsPinned: true,
      onKeepMentionedAgentsPinnedChange: (value) => pinnedChanges.push(value),
    }),
  );

  // fireEvent returns false when the dispatched event was canceled, which is
  // what keeps a contenteditable from losing focus on mousedown.
  assert.equal(
    fireEvent.mouseDown(view.getByTestId("mention-autocomplete")),
    false,
  );

  const options = view.getByRole("button", { name: "Options" });
  assert.equal(fireEvent.mouseDown(options.parentElement), false);

  // A label hands focus to its control from the click default action, which
  // no mousedown guard can cancel, so the label drives the switch itself.
  fireEvent.click(options);
  assert.equal(
    fireEvent.click(view.getByText("Automatically mention agents")),
    false,
  );
  assert.deepEqual(pinnedChanges, [false]);

  // The row buttons must keep selecting: preventing mousedown on the
  // containers, not pointerdown, leaves their compatibility events intact.
  fireEvent.mouseDown(view.getByRole("button", { name: "Mention Agent Ada" }));
  assert.deepEqual(selected, [mention]);
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
    showMentionAgentProvenanceMarker(suggestion("managed-here"), true),
    false,
  );
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-elsewhere"), true),
    true,
  );
});

test("unique agents omit management provenance", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion("managed-here"), false),
    false,
  );
});

test("agents without trustworthy provenance omit management provenance", () => {
  assert.equal(
    showMentionAgentProvenanceMarker(suggestion(undefined), true),
    false,
  );
});
