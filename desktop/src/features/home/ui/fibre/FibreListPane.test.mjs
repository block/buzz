/**
 * Fibre list rendering: kind labels, Open/Done tabs, seen dots, and empty
 * states. Mounts the shipping FibreListPane rather than reimplementing layout.
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

function renderList(props = {}) {
  const tabs = [];
  const sorts = [];
  const selected = [];
  render(
    createElement(FibreListPane, {
      doneCount: 0,
      fibres: [fibre()],
      listTab: "open",
      nowMs: NOW,
      onListTabChange: (tab) => tabs.push(tab),
      onSelect: (id) => selected.push(id),
      onSortChange: (sort) => sorts.push(sort),
      openCount: 1,
      selectedId: "f1",
      sort: "priority",
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
    openCount: 2,
  });

  assert.equal(screen.getByTestId("fibre-tab-open-count").textContent, "2");
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

test("empty open list omits Inbox Zero copy and keeps the count on the tab", () => {
  renderList({
    fibres: [],
    openCount: 0,
    selectedId: null,
  });

  assert.equal(
    screen.getByTestId("fibre-list").textContent.includes("Inbox Zero"),
    false,
  );
  assert.equal(screen.getByTestId("fibre-tab-open-count").textContent, "0");
  assert.equal(screen.queryAllByTestId("fibre-row").length, 0);
});

test("empty done list shows completed copy", () => {
  renderList({
    doneCount: 0,
    fibres: [],
    listTab: "done",
    openCount: 0,
    selectedId: null,
  });

  assert.match(
    screen.getByTestId("fibre-list").textContent,
    /Nothing completed yet/,
  );
  assert.equal(screen.getByTestId("fibre-tab-done-count").textContent, "0");
});

test("unseen fibres show a blue dot; updated fibres show purple", () => {
  const item = fibre({ updatedAt: 50 });
  renderList({
    fibres: [
      item,
      fibre({ id: "f2", title: "Already opened", updatedAt: 50 }),
      fibre({ id: "f3", title: "Updated after open", updatedAt: 80 }),
    ],
    openCount: 3,
    seenAtById: { f2: 50, f3: 50 },
    selectedId: "f1",
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
    doneCount: 1,
    fibres: [fibre({ status: "done" })],
    listTab: "done",
    openCount: 0,
  });

  assert.equal(screen.queryByTestId("fibre-seen-dot"), null);
});

test("Open and Done tabs notify the parent", () => {
  const { tabs } = renderList();
  fireEvent.click(screen.getByTestId("fibre-tab-done"));
  assert.deepEqual(tabs, ["done"]);
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
