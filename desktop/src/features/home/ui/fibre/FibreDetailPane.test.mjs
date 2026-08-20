/**
 * Fibre detail pane: Fibre Zero empty state and Done from a fixture fibre.
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
        isZero: true,
        nowMs: NOW,
        onDismiss: () => {},
        onDone: () => {},
        onOpenContext: () => {},
        onRestore: () => {},
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

test("Fibre Zero offers restore", () => {
  const restored = [];
  renderPane({
    onRestore: () => restored.push(true),
  });

  assert.ok(screen.getByTestId("fibre-zero"));
  assert.match(screen.getByTestId("fibre-zero").textContent, /Fibre Zero/);
  fireEvent.click(screen.getByTestId("fibre-restore"));
  assert.equal(restored.length, 1);
});
