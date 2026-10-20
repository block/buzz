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

function systemMessage({ actor, createdAt = 1, id, reactions = [], target }) {
  return {
    author: "System",
    body: JSON.stringify({ type: "member_joined", actor, target }),
    createdAt,
    depth: 0,
    id,
    kind: 40099,
    reactions,
    time: "12:00 PM",
  };
}

function memberLeftMessage({ actor, createdAt = 1, id, reactions = [] }) {
  return {
    author: "System",
    body: JSON.stringify({ type: "member_left", actor }),
    createdAt,
    depth: 0,
    id,
    kind: 40099,
    reactions,
    time: "12:00 PM",
  };
}

function reaction(emoji, { reactedByCurrentUser = false } = {}) {
  return {
    emoji,
    count: 1,
    reactedByCurrentUser,
    users: [{ avatarUrl: null, displayName: "Zed", pubkey: "99".repeat(32) }],
  };
}

function profileFor(pubkey, displayName) {
  return {
    [pubkey]: {
      avatarUrl: null,
      displayName,
      isAgent: false,
      name: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  };
}

function normalizeText(text) {
  return text.replace(/\s+/g, " ").trim();
}

async function renderSystemMessageRow({
  currentPubkey = "viewer",
  groupedMessages,
  message = groupedMessages[0],
  onToggleReaction,
  profiles,
}) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const { SystemMessageRow } = await import("./SystemMessageRow.tsx");
  const queryClient = new QueryClient();

  render(
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        TooltipProvider,
        null,
        createElement(SystemMessageRow, {
          currentPubkey,
          groupedMessages,
          message,
          onToggleReaction,
          profiles,
        }),
      ),
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

test("grouped duplicate mixed-mechanism arrivals render singular neutral copy", async () => {
  const { screen } = await import("@testing-library/react");
  const viewer = "10".repeat(32);
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: viewer, createdAt: 2, id: "b", target: elrond }),
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
    },
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(normalizeText(row.textContent ?? ""), "Elrond arrived");
});

// --- joined-then-left lifecycle groups ---------------------------------------
//
// `buildTimelineItems` groups every contiguous equivalent self-arrival with the
// matching departure, so the renderer must describe N>=1 arrivals + 1 departure.
// When it cannot, `SystemMessageRow` falls back to the group's *oldest* message
// and the departure never renders — the timeline claims a departed member is
// still present.

test("a duplicate self-join followed by leaving renders one lifecycle summary", async () => {
  const { screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: elrond, createdAt: 2, id: "b", target: elrond }),
    memberLeftMessage({ actor: elrond, createdAt: 3, id: "c" }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
    profiles: profileFor(elrond, "Elrond"),
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond joined, then left the channel",
  );
});

test("more than two duplicate self-joins before leaving still render one lifecycle summary", async () => {
  const { screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: elrond, createdAt: 2, id: "b", target: elrond }),
    systemMessage({ actor: elrond, createdAt: 3, id: "c", target: elrond }),
    memberLeftMessage({ actor: elrond, createdAt: 4, id: "d" }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
    profiles: profileFor(elrond, "Elrond"),
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond joined, then left the channel",
  );
});

test("a single self-join followed by leaving keeps the two-event lifecycle copy", async () => {
  const { screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({ actor: elrond, createdAt: 1, id: "a", target: elrond }),
    memberLeftMessage({ actor: elrond, createdAt: 2, id: "b" }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
    profiles: profileFor(elrond, "Elrond"),
  });

  const row = screen.getByTestId("system-message-row");
  assert.equal(
    normalizeText(row.textContent ?? ""),
    "Elrond joined, then left the channel",
  );
});

test("an addition before the departing member's self-join is not a lifecycle summary", async () => {
  const { screen } = await import("@testing-library/react");
  const viewer = "10".repeat(32);
  const elrond = "11".repeat(32);
  // `buildTimelineItems` never groups this shape — an addition cannot precede a
  // departure in one group. Hand-built here so the arrivals guard stays
  // falsifiable: dropping it would let "was added" collapse into "joined, then
  // left", attributing a self-join the member never made.
  const groupedMessages = [
    systemMessage({ actor: viewer, createdAt: 1, id: "a", target: elrond }),
    systemMessage({ actor: elrond, createdAt: 2, id: "b", target: elrond }),
    memberLeftMessage({ actor: elrond, createdAt: 3, id: "c" }),
  ];

  await renderSystemMessageRow({
    currentPubkey: viewer,
    groupedMessages,
    profiles: profileFor(elrond, "Elrond"),
  });

  const row = screen.getByTestId("system-message-row");
  assert.notEqual(
    normalizeText(row.textContent ?? ""),
    "Elrond joined, then left the channel",
  );
});

test("a duplicate self-join lifecycle group surfaces reactions from every source event", async () => {
  const { screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({
      actor: elrond,
      createdAt: 1,
      id: "a",
      reactions: [reaction("👍")],
      target: elrond,
    }),
    systemMessage({
      actor: elrond,
      createdAt: 2,
      id: "b",
      reactions: [reaction("🚀")],
      target: elrond,
    }),
    memberLeftMessage({
      actor: elrond,
      createdAt: 3,
      id: "c",
      reactions: [reaction("🎉")],
    }),
  ];

  await renderSystemMessageRow({
    groupedMessages,
    onToggleReaction: async () => {},
    profiles: profileFor(elrond, "Elrond"),
  });

  for (const emoji of ["👍", "🚀", "🎉"]) {
    assert.ok(
      screen.queryByText(emoji),
      `expected the grouped row to surface ${emoji}`,
    );
  }
});

test("removing a reaction from a duplicate self-join lifecycle group targets every reacted source", async () => {
  const { act, fireEvent, screen } = await import("@testing-library/react");
  const elrond = "11".repeat(32);
  const groupedMessages = [
    systemMessage({
      actor: elrond,
      createdAt: 1,
      id: "a",
      reactions: [reaction("👍", { reactedByCurrentUser: true })],
      target: elrond,
    }),
    systemMessage({ actor: elrond, createdAt: 2, id: "b", target: elrond }),
    memberLeftMessage({
      actor: elrond,
      createdAt: 3,
      id: "c",
      reactions: [reaction("👍", { reactedByCurrentUser: true })],
    }),
  ];
  const removals = [];

  await renderSystemMessageRow({
    groupedMessages,
    onToggleReaction: async (source, emoji, remove) => {
      if (remove) removals.push([source.id, emoji]);
    },
    profiles: profileFor(elrond, "Elrond"),
  });

  // `act` (not a bare microtask drain) so the removal's optimistic state update
  // and its awaited fan-out across every reacted source both settle before the
  // assertion — a bare `fireEvent` leaves that update outside act.
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Toggle 👍 reaction" }));
  });

  assert.deepEqual(removals.sort(), [
    ["a", "👍"],
    ["c", "👍"],
  ]);
});
