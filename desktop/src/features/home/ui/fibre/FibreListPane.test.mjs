/**
 * Fibre list rendering: kind labels, lane tabs, seen dots, and empty states.
 * Mounts the shipping FibreListPane rather than reimplementing layout.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let FibreListPane;

const NOW = 1_700_000_000_000;

function fibre(overrides = {}) {
  return {
    id: "f1",
    kind: "blocker",
    status: "open",
    score: 98,
    engagement: 10,
    lane: "important",
    title: "Incident root cause is identified",
    summary: "The agent traced the degradation.",
    why: "An agent @-mentioned you.",
    whyShort: "Unanswered agent @mention.",
    signals: [{ weight: "+34", label: "Direct @mention" }],
    channelId: "war-room",
    channelName: "war-room",
    isDm: false,
    people: [{ pubkey: "aa", label: "Incident Responder" }],
    artifacts: [
      {
        eventId: "evt-1",
        channelId: "war-room",
        channelName: "war-room",
        threadRootId: "evt-1",
        authorPubkey: "aa",
        authorLabel: "Incident Responder",
        content: "FINDINGS",
        createdAt: 1_700_000_000 - 41 * 60,
        isDm: false,
      },
    ],
    createdAt: 1_700_000_000 - 41 * 60,
    updatedAt: 1_700_000_000 - 41 * 60,
    ...overrides,
  };
}

function counts(overrides = {}) {
  return { important: 1, hot: 0, other: 0, done: 0, ...overrides };
}

function renderList(props = {}) {
  const tabs = [];
  const sorts = [];
  const selected = [];
  render(
    createElement(FibreListPane, {
      fibres: [fibre()],
      isInboxZero: false,
      listTab: "important",
      nowMs: NOW,
      onListTabChange: (tab) => tabs.push(tab),
      onSelect: (id) => selected.push(id),
      onSortChange: (sort) => sorts.push(sort),
      selectedId: "f1",
      sort: "priority",
      tabCounts: counts(),
      ...props,
    }),
  );
  return { selected, sorts, tabs };
}

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.setPointerCapture = () => {};
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  dom.window.matchMedia = () => ({
    matches: true,
    addEventListener() {},
    removeEventListener() {},
  });

  ({ cleanup, fireEvent, render, screen } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ FibreListPane } = await import("./FibreListPane.tsx"));
});

afterEach(() => cleanup?.());
after(() => dom.window.close());

test("renders kind labels and titles without a score badge", () => {
  const { selected } = renderList({
    fibres: [
      fibre(),
      fibre({
        id: "f2",
        kind: "ask",
        score: 84,
        title: "Vlad needs you to run the scripts",
        whyShort: "Unanswered instruction",
        channelName: "hack-project-mesh",
      }),
    ],
    tabCounts: counts({ important: 2 }),
  });

  assert.equal(
    screen.getByTestId("fibre-tab-important-count").textContent,
    "2",
  );
  assert.equal(screen.getByTestId("fibre-tab-done-count").textContent, "0");
  const rows = screen.getAllByTestId("fibre-row");
  assert.equal(rows.length, 2);
  assert.equal(rows[0].getAttribute("data-kind"), "blocker");
  assert.equal(rows[1].getAttribute("data-kind"), "ask");
  assert.match(rows[0].textContent, /Blocker/);
  assert.match(rows[0].textContent, /Incident root cause/);
  assert.equal(rows[0].textContent.includes("98"), false);
  assert.match(rows[1].textContent, /Ask/);

  const kindLabel = rows[0].querySelector("span.text-sm.font-medium");
  assert.ok(kindLabel);
  assert.match(
    kindLabel.getAttribute("style") ?? "",
    /232,\s*129,\s*112|#E88170/i,
  );

  fireEvent.click(rows[1]);
  assert.deepEqual(selected, ["f2"]);
});

test("all four lanes render with their counts", () => {
  renderList({
    tabCounts: counts({ important: 2, hot: 5, other: 9, done: 3 }),
  });

  assert.deepEqual(
    ["important", "hot", "other", "done"].map(
      (tab) => screen.getByTestId(`fibre-tab-${tab}-count`).textContent,
    ),
    ["2", "5", "9", "3"],
  );
  assert.equal(
    screen.getByTestId("fibre-tab-important").getAttribute("aria-selected"),
    "true",
  );
});

test("an empty lane explains itself while other lanes still have fibres", () => {
  renderList({
    fibres: [],
    listTab: "hot",
    selectedId: null,
    tabCounts: counts({ important: 4, hot: 0 }),
  });

  assert.match(
    screen.getByTestId("fibre-lane-empty").textContent,
    /No live discussions/,
  );
});

test("at inbox zero the lane copy yields to the wallpaper", () => {
  renderList({
    fibres: [],
    isInboxZero: true,
    selectedId: null,
    tabCounts: counts({ important: 0 }),
  });

  assert.equal(screen.queryByTestId("fibre-lane-empty"), null);
  assert.equal(screen.queryAllByTestId("fibre-row").length, 0);
});

test("empty done list shows completed copy even at inbox zero", () => {
  renderList({
    fibres: [],
    isInboxZero: true,
    listTab: "done",
    selectedId: null,
    tabCounts: counts({ important: 0 }),
  });

  assert.match(
    screen.getByTestId("fibre-lane-empty").textContent,
    /Nothing completed yet/,
  );
});

test("unseen fibres show a blue dot; updated fibres show purple", () => {
  const item = fibre({ updatedAt: 50 });
  renderList({
    fibres: [
      item,
      fibre({ id: "f2", title: "Already opened", updatedAt: 50 }),
      fibre({ id: "f3", title: "Updated after open", updatedAt: 80 }),
    ],
    seenAtById: { f2: 50, f3: 50 },
    selectedId: "f1",
    tabCounts: counts({ important: 3 }),
  });

  const rows = screen.getAllByTestId("fibre-row");
  assert.equal(
    rows[0].querySelector("[data-state=unseen]")?.getAttribute("aria-label"),
    "Unread",
  );
  assert.equal(rows[1].querySelector("[data-testid=fibre-seen-dot]"), null);
  assert.equal(
    rows[2].querySelector("[data-state=updated]")?.getAttribute("aria-label"),
    "Updated",
  );
});

test("done tab does not show seen dots", () => {
  renderList({
    fibres: [fibre({ status: "done" })],
    listTab: "done",
    tabCounts: counts({ important: 0, done: 1 }),
  });

  assert.equal(screen.queryByTestId("fibre-seen-dot"), null);
});

test("every tab notifies the parent", () => {
  const { tabs } = renderList();
  for (const tab of ["hot", "other", "done"]) {
    fireEvent.click(screen.getByTestId(`fibre-tab-${tab}`));
  }
  assert.deepEqual(tabs, ["hot", "other", "done"]);
});

test("sort trigger is available in the list header", () => {
  renderList();
  assert.ok(screen.getByTestId("fibre-sort-trigger"));
});

const ALICE = "a".repeat(64);

test("people line uses profile display names instead of stored pubkeys", () => {
  renderList({
    fibres: [
      fibre({
        people: [
          {
            pubkey: ALICE,
            label: `${ALICE.slice(0, 8)}…${ALICE.slice(-4)}`,
          },
        ],
      }),
    ],
    profiles: {
      [ALICE]: {
        displayName: "Alice",
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  assert.match(screen.getByTestId("fibre-row").textContent, /Alice/);
  assert.equal(
    screen.getByTestId("fibre-row").textContent.includes(ALICE.slice(0, 8)),
    false,
  );
});
