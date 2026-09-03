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

async function renderSystemMessageRow({
  currentPubkey = "viewer",
  groupedMessages,
  message = groupedMessages[0],
  profiles,
}) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { SystemMessageRow } = await import("./SystemMessageRow.tsx");
  const queryClient = new QueryClient();

  render(
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(SystemMessageRow, {
        currentPubkey,
        groupedMessages,
        message,
        profiles,
      }),
    ),
  );
}

test("grouped duplicate arrival targets render one unique added member", async () => {
  const { screen } = await import("@testing-library/react");
  const target = "11".repeat(32);
  const firstActor = "12".repeat(32);
  const secondActor = "13".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: firstActor, createdAt: 1, id: "a", target }),
    systemMessage({ actor: secondActor, createdAt: 2, id: "b", target }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
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
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(normalizeText(row.textContent ?? ""), "Elrond was added");
  assert.equal(
    screen
      .getByTestId("system-message-avatar-stack")
      .getAttribute("aria-label"),
    "1 channel member",
  );
});

test("grouped duplicate arrival targets keep viewer grammar truthful", async () => {
  const { screen } = await import("@testing-library/react");
  const viewer = "10".repeat(32);
  const firstActor = "11".repeat(32);
  const secondActor = "12".repeat(32);
  const groupedMessages = [
    systemMessage({
      actor: firstActor,
      createdAt: 1,
      id: "a",
      target: viewer,
    }),
    systemMessage({
      actor: secondActor,
      createdAt: 2,
      id: "b",
      target: viewer,
    }),
  ];

  await renderSystemMessageRow({
    currentPubkey: viewer,
    groupedMessages,
    profiles: {
      [viewer]: {
        avatarUrl: null,
        displayName: "Viewer",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(normalizeText(row.textContent ?? ""), "You were added");
});

test("grouped mixed additions render mechanism-truthful copy", async () => {
  const { screen } = await import("@testing-library/react");
  const viewer = "10".repeat(32);
  const elrond = "11".repeat(32);
  const legolas = "12".repeat(32);
  const gimli = "13".repeat(32);
  const gandalf = "14".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: viewer, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: elrond, createdAt: 2, id: "b", target: legolas }),
    systemMessage({ actor: elrond, createdAt: 3, id: "c", target: gimli }),
    systemMessage({ actor: viewer, createdAt: 4, id: "d", target: gandalf }),
  ];

  await renderSystemMessageRow({
    currentPubkey: viewer,
    groupedMessages,
    profiles: {
      [elrond]: {
        avatarUrl: null,
        displayName: "Elrond",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
      [legolas]: {
        avatarUrl: null,
        displayName: "Legolas",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
      [gimli]: {
        avatarUrl: null,
        displayName: "Gimli",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
      [gandalf]: {
        avatarUrl: null,
        displayName: "Gandalf",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond was added along with Legolas, Gimli, and Gandalf",
  );
});

test("grouped self-joins render joined copy", async () => {
  const { screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const legolas = "12".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: legolas, createdAt: 2, id: "b", target: legolas }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
    profiles: {
      [elrond]: {
        avatarUrl: null,
        displayName: "Elrond",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
      [legolas]: {
        avatarUrl: null,
        displayName: "Legolas",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond joined along with Legolas",
  );
});

test("grouped duplicate self-joins render singular joined copy", async () => {
  const { screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: elrond, createdAt: 2, id: "b", target: elrond }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
    profiles: {
      [elrond]: {
        avatarUrl: null,
        displayName: "Elrond",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(normalizeText(row.textContent ?? ""), "Elrond joined");
});

test("grouped self-joins plus additions render neutral arrival copy", async () => {
  const { screen } = await import("@testing-library/react");
  const viewer = "10".repeat(32);
  const elrond = "11".repeat(32);
  const legolas = "12".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: viewer, createdAt: 2, id: "b", target: legolas }),
  ];

  await renderSystemMessageRow({
    currentPubkey: viewer,
    groupedMessages,
    profiles: {
      [elrond]: {
        avatarUrl: null,
        displayName: "Elrond",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
      [legolas]: {
        avatarUrl: null,
        displayName: "Legolas",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond arrived along with Legolas",
  );
});
