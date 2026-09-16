import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import { installLocalStorage } from "../lib/testStorage.mjs";

const card = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "4455",
  sid: "demo-1",
  stack: "hula-app",
};

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    ResizeObserver: class {
      observe() {}
      disconnect() {}
    },
    self: dom.window,
    window: dom.window,
  });
  installLocalStorage(dom.window.localStorage);
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetPlaygroundState } = await import("../lib/sessions.ts");
  resetPlaygroundState();
  globalThis.localStorage?.clear();
  delete globalThis.__BUZZ_PLAYGROUND_PROBE__;
  delete globalThis.__BUZZ_PLAYGROUND_OPEN_URL__;
  delete dom.window.__TAURI_INTERNALS__;
  delete globalThis.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

async function renderCard(cardData = card) {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { PlaygroundCard } = await import("./PlaygroundCard.tsx");
  const {
    Outlet,
    RouterProvider,
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
  } = await import("@tanstack/react-router");
  const { configurePlaygroundScope } = await import("../lib/sessions.ts");
  configurePlaygroundScope("pub", "wss://relay.example.com");
  const rootRoute = createRootRoute({
    component: Outlet,
  });
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/",
    component: () => createElement(PlaygroundCard, { card: cardData }),
  });
  const router = createRouter({
    defaultPendingMs: 0,
    routeTree: rootRoute.addChildren([indexRoute]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  render(createElement(RouterProvider, { router }));
  return screen;
}

test("Pin / Open / Open as Split sit below the text and right-justify", async () => {
  const screen = await renderCard();
  const cardEl = screen.getByTestId("playground-card");
  assert.equal(cardEl.getAttribute("data-orientation"), "horizontal");
  assert.match(cardEl.className, /items-start/);
  const name = screen.getByTestId("playground-card-name");
  const url = screen.getByTestId("playground-card-url");
  const pin = screen.getByTestId("playground-card-pin-action");
  const open = screen.getByTestId("playground-card-open");
  const split = screen.getByTestId("playground-card-open-split");
  assert.equal(open.textContent, "Open");
  assert.equal(pin.textContent, "Pin");
  assert.equal(split.textContent, "Open as Split");
  assert.equal(split.disabled, true);
  const body = name.closest("[data-slot='attachment-content']");
  const actions = open.closest("[data-slot='attachment-actions']");
  assert.ok(body);
  assert.ok(actions);
  assert.ok(body.contains(url));
  assert.ok(actions.contains(pin));
  assert.ok(actions.contains(open));
  assert.ok(actions.contains(split));
  assert.match(actions.className, /justify-end/);
  assert.equal(body.parentElement, actions.parentElement);
  assert.notEqual(actions.parentElement, cardEl);
  assert.match(actions.parentElement.className, /flex-col/);
  assert.ok(body.compareDocumentPosition(actions) & 4);
});

test("Open probes first: down means toast and no ghost row", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  const { listPlaygroundSessions } = await import("../lib/sessions.ts");
  const screen = await renderCard();

  assert.equal(screen.getByTestId("playground-card-name").textContent, "Demo");
  assert.match(
    screen.getByTestId("playground-card-pin").textContent ?? "",
    /4455/,
  );

  globalThis.__BUZZ_PLAYGROUND_PROBE__ = () => ({
    up: false,
    status: 502,
    message: "bad gateway",
  });

  await fireEvent.click(screen.getByTestId("playground-card-open"));
  await waitFor(() => assert.equal(listPlaygroundSessions().length, 0));
  await waitFor(() =>
    assert.equal(screen.getByTestId("playground-card-open").disabled, false),
  );
});

test("Pin on a new sid probes and creates a session", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  const { listPlaygroundSessions } = await import("../lib/sessions.ts");
  const screen = await renderCard();
  let probed = 0;
  globalThis.__BUZZ_PLAYGROUND_PROBE__ = () => {
    probed += 1;
    return { up: true, status: 200 };
  };

  await fireEvent.click(screen.getByTestId("playground-card-pin-action"));
  await waitFor(() => assert.equal(listPlaygroundSessions().length, 1));
  assert.equal(probed, 1);
});

test("Pin on an existing sid shows it and does not probe again", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  const {
    addPlaygroundSession,
    dismissPlayground,
    getActivePlaygroundSid,
    listPlaygroundSessions,
  } = await import("../lib/sessions.ts");
  const screen = await renderCard();

  addPlaygroundSession(card);
  dismissPlayground();
  assert.equal(getActivePlaygroundSid(), null);
  assert.equal(listPlaygroundSessions().length, 1);

  let probed = 0;
  globalThis.__BUZZ_PLAYGROUND_PROBE__ = () => {
    probed += 1;
    return { up: true, status: 200 };
  };

  await fireEvent.click(screen.getByTestId("playground-card-pin-action"));
  await waitFor(() => assert.equal(getActivePlaygroundSid(), "demo-1"));
  assert.equal(listPlaygroundSessions().length, 1);
  assert.equal(probed, 0);
});

test("URL is an anchor that opens the browser and does not add a session", async () => {
  const { fireEvent } = await import("@testing-library/react");
  const { listPlaygroundSessions } = await import("../lib/sessions.ts");
  const screen = await renderCard();
  let probed = 0;
  globalThis.__BUZZ_PLAYGROUND_PROBE__ = () => {
    probed += 1;
    return { up: true, status: 200 };
  };
  const opened = [];
  globalThis.__BUZZ_PLAYGROUND_OPEN_URL__ = (url) => {
    opened.push(url);
  };

  const url = screen.getByTestId("playground-card-url");
  assert.equal(url.tagName, "A");
  assert.equal(url.getAttribute("href"), "https://app.example.com");
  await fireEvent.click(url);
  assert.equal(probed, 0);
  assert.equal(listPlaygroundSessions().length, 0);
  assert.deepEqual(opened, ["https://app.example.com"]);
});

test("PIN copy button writes the pin, not the URL", async () => {
  const { fireEvent, waitFor } = await import("@testing-library/react");
  const screen = await renderCard();
  let copied = null;
  const internals = {
    invoke: async (cmd, args) => {
      if (cmd === "copy_text_to_clipboard") {
        copied = args.text;
        return;
      }
      throw new Error(`unmocked Tauri command: ${cmd}`);
    },
    transformCallback: () => Math.random(),
  };
  dom.window.__TAURI_INTERNALS__ = internals;
  globalThis.__TAURI_INTERNALS__ = internals;

  await fireEvent.click(screen.getByTestId("playground-card-copy-pin"));
  await waitFor(() => assert.equal(copied, "4455"));
});

test("PIN is hidden when omitted and Open still works", async () => {
  const { pin: _pin, ...withoutPin } = card;
  const screen = await renderCard(withoutPin);
  assert.equal(screen.queryByTestId("playground-card-pin"), null);
  assert.equal(screen.queryByTestId("playground-card-copy-pin"), null);
  assert.equal(screen.getByTestId("playground-card-open").textContent, "Open");
});
