import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Element: dom.window.Element,
    getComputedStyle: dom.window.getComputedStyle,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    SVGElement: dom.window.SVGElement,
    window: dom.window,
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

function systemMessage({ actor, createdAt = 1, id, target }) {
  return {
    author: "System",
    body: JSON.stringify({ type: "member_joined", actor, target }),
    createdAt,
    depth: 0,
    id,
    kind: 40099,
    reactions: [],
    time: "12:00 PM",
  };
}

function normalizeText(text) {
  return text.replace(/\s+/g, " ").trim();
}

test("grouped duplicate arrival targets render one unique member", async () => {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { SystemMessageRow } = await import("./SystemMessageRow.tsx");
  const queryClient = new QueryClient();

  const target = "11".repeat(32);
  const firstActor = "12".repeat(32);
  const secondActor = "13".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: firstActor, createdAt: 1, id: "a", target }),
    systemMessage({ actor: secondActor, createdAt: 2, id: "b", target }),
  ];

  render(
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(SystemMessageRow, {
        currentPubkey: "viewer",
        groupedMessages,
        message: groupedMessages[0],
        profiles: {
          [target]: {
            avatarUrl: null,
            displayName: "Elrond",
            isAgent: false,
            name: null,
            nip05Handle: null,
            ownerPubkey: null,
          },
        },
      }),
    ),
  );

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond joined the channel",
  );
  assert.equal(
    screen
      .getByTestId("system-message-avatar-stack")
      .getAttribute("aria-label"),
    "1 channel member",
  );
});
