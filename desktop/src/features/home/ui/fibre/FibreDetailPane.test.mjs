/**
 * Fibre detail pane: empty open inbox and Done from a fixture fibre.
 * Keyboard Done is covered in the fibre-inbox Playwright spec.
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
let QueryClient;
let QueryClientProvider;
let FibreDetailPane;

const NOW = 1_700_000_000_000;

let lastClient;

function renderPane(props) {
  lastClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    createElement(
      QueryClientProvider,
      { client: lastClient },
      createElement(FibreDetailPane, {
        fibre: null,
        listTab: "open",
        nowMs: NOW,
        onDismiss: () => {},
        onDone: () => {},
        onOpenContext: () => {},
        onReopen: () => {},
        ...props,
      }),
    ),
  );
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
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ FibreDetailPane } = await import("./FibreDetailPane.tsx"));
});

afterEach(() => {
  cleanup?.();
  lastClient?.clear();
  lastClient?.cancelQueries();
});
after(() => dom.window.close());

test("empty open inbox has no Inbox Zero copy", () => {
  renderPane();

  assert.ok(screen.getByTestId("fibre-zero"));
  assert.equal(screen.getByTestId("fibre-zero").textContent, "");
  assert.equal(screen.queryByTestId("fibre-restore"), null);
});

test("Done shortcut inherits button color", () => {
  const fibre = {
    id: "f1",
    kind: "ask",
    status: "open",
    score: 90,
    title: "Threadception Inquiry",
    summary: "A question about the thread.",
    why: "Unanswered ask.",
    whyShort: "Unanswered ask.",
    signals: [],
    channelId: "c1",
    channelName: "general",
    isDm: false,
    people: [],
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    artifacts: [],
  };

  renderPane({
    fibre,
  });

  const kbd = screen.getByTestId("fibre-done-kbd");
  assert.equal(kbd.className.includes("text-muted-foreground"), false);
  assert.equal(kbd.className.includes("text-current"), true);
  assert.ok(screen.getByTestId("fibre-artifacts"));
  assert.equal(
    screen.getByTestId("fibre-artifacts").className.includes("max-h-"),
    false,
  );
});

test("completed fibre offers Reopen instead of Done", () => {
  const reopened = [];
  const fibre = {
    id: "f1",
    kind: "ask",
    status: "done",
    score: 90,
    title: "Threadception Inquiry",
    summary: "A question about the thread.",
    why: "Unanswered ask.",
    whyShort: "Unanswered ask.",
    signals: [],
    channelId: "c1",
    channelName: "general",
    isDm: false,
    people: [],
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    artifacts: [],
  };

  renderPane({
    fibre,
    listTab: "done",
    onReopen: (item) => reopened.push(item.id),
  });

  assert.equal(screen.queryByTestId("fibre-done"), null);
  fireEvent.click(screen.getByTestId("fibre-reopen"));
  assert.deepEqual(reopened, ["f1"]);
});
