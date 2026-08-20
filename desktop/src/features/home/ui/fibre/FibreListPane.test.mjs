/**
 * Fibre list rendering: kind colors, score chips, and the Fibre Zero empty
 * state. Mounts the shipping FibreListPane rather than reimplementing layout.
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

test("renders kind-colored score chips and titles", () => {
  const selected = [];
  render(
    createElement(FibreListPane, {
      clearedCount: 3,
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
      nowMs: NOW,
      onSelect: (id) => selected.push(id),
      selectedId: "f1",
    }),
  );

  assert.match(
    screen.getByTestId("fibre-list").textContent,
    /2 open · 3 cleared/,
  );
  const rows = screen.getAllByTestId("fibre-row");
  assert.equal(rows.length, 2);
  assert.equal(rows[0].getAttribute("data-kind"), "blocker");
  assert.equal(rows[1].getAttribute("data-kind"), "ask");
  assert.match(rows[0].textContent, /Incident root cause/);
  assert.match(rows[0].textContent, /98/);

  const scoreChip = rows[0].querySelector("span");
  assert.ok(scoreChip);
  assert.match(
    scoreChip.getAttribute("style") ?? "",
    /232,\s*129,\s*112|#E88170/i,
  );

  const askChip = rows[1].querySelector("span");
  assert.ok(askChip);
  assert.match(
    askChip.getAttribute("style") ?? "",
    /229,\s*185,\s*47|#E5B92F/i,
  );

  fireEvent.click(rows[1]);
  assert.deepEqual(selected, ["f2"]);
});

test("empty list shows Fibre Zero copy", () => {
  render(
    createElement(FibreListPane, {
      clearedCount: 12,
      fibres: [],
      nowMs: NOW,
      onSelect: () => {},
      selectedId: null,
    }),
  );

  assert.match(screen.getByTestId("fibre-list").textContent, /Fibre Zero/);
  assert.match(
    screen.getByTestId("fibre-list").textContent,
    /0 open · 12 cleared/,
  );
  assert.equal(screen.queryAllByTestId("fibre-row").length, 0);
});

const ALICE = "a".repeat(64);

test("people line uses profile display names instead of stored pubkeys", () => {
  render(
    createElement(FibreListPane, {
      clearedCount: 0,
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
      nowMs: NOW,
      onSelect: () => {},
      profiles: {
        [ALICE]: {
          displayName: "Alice",
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
        },
      },
      selectedId: "f1",
    }),
  );

  assert.match(screen.getByTestId("fibre-row").textContent, /Alice/);
  assert.equal(
    screen.getByTestId("fibre-row").textContent.includes(ALICE.slice(0, 8)),
    false,
  );
});
