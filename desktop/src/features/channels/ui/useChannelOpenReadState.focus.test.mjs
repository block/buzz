import assert from "node:assert/strict";
import { test } from "node:test";

import { JSDOM } from "jsdom";
import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useChannelOpenReadState } from "./useChannelOpenReadState.ts";
import { AppShellProvider } from "@/app/AppShellContext";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;
const originalHTMLElement = globalThis.HTMLElement;
const originalActEnvironment = globalThis.IS_REACT_ACT_ENVIRONMENT;

function installFocusDom() {
  const dom = new JSDOM("<!doctype html><html><body><div id='root'></div></body></html>");
  let focused = true;
  Object.defineProperty(dom.window.document, "hasFocus", {
    configurable: true,
    value: () => focused,
  });
  Object.defineProperty(dom.window.document, "visibilityState", {
    configurable: true,
    value: "visible",
  });
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  return {
    dom,
    blur() {
      focused = false;
    },
    focus() {
      focused = true;
    },
  };
}

test("passive read marking is gated on app focus", async () => {
  const stub = installFocusDom();
  const markedRead = [];
  const undone = [];

  function Harness({ channelId, readAt }) {
    useChannelOpenReadState(channelId, true, readAt);
    return null;
  }

  const shellValue = {
    feedItemState: { undoUnread: (id) => undone.push(id) },
    locallyUnreadFeedItems: [],
    markChannelRead: (channelId, readAt, options) =>
      markedRead.push([channelId, readAt, options]),
  };

  const root = createRoot(document.getElementById("root"));

  await act(async () =>
    root.render(
      React.createElement(
        AppShellProvider,
        { value: shellValue },
        React.createElement(Harness, { channelId: "general", readAt: "2026-09-07T22:58:33.000Z" }),
      ),
    ),
  );
  assert.deepEqual(markedRead, [
    ["general", "2026-09-07T22:58:33.000Z", { topLevelOnly: true }],
  ]);

  // Window loses focus. A newer top-level message arrives and activeReadAt
  // advances — the marker must NOT advance while unfocused.
  stub.blur();
  await act(async () =>
    window.dispatchEvent(new window.Event("blur")),
  );
  await act(async () =>
    root.render(
      React.createElement(
        AppShellProvider,
        { value: shellValue },
        React.createElement(Harness, { channelId: "general", readAt: "2026-09-07T22:58:40.000Z" }),
      ),
    ),
  );
  assert.equal(
    markedRead.length,
    1,
    "read marker must not advance while the window is unfocused",
  );

  // Window regains focus — the effect re-runs and catches the marker up
  // in one tick.
  stub.focus();
  await act(async () =>
    window.dispatchEvent(new window.Event("focus")),
  );
  await act(
    async () => new Promise((resolve) => window.setTimeout(resolve, 10)),
  );
  assert.deepEqual(markedRead, [
    ["general", "2026-09-07T22:58:33.000Z", { topLevelOnly: true }],
    ["general", "2026-09-07T22:58:40.000Z", { topLevelOnly: true }],
  ]);

  await act(async () => root.unmount());

  if (originalDocument === undefined) delete globalThis.document;
  else globalThis.document = originalDocument;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
  if (originalHTMLElement === undefined) delete globalThis.HTMLElement;
  else globalThis.HTMLElement = originalHTMLElement;
  if (originalActEnvironment === undefined) delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  else globalThis.IS_REACT_ACT_ENVIRONMENT = originalActEnvironment;
  stub.dom.window.close();
});
