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
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const message = {
  id: "thread-root",
  createdAt: 1,
  pubkey: "author",
  author: "Author",
  avatarUrl: null,
  role: undefined,
  personaDisplayName: undefined,
  time: "12:00 PM",
  body: "Thread root",
  parentId: null,
  rootId: null,
  depth: 0,
  accent: false,
  pending: undefined,
  edited: false,
  kind: 9,
  tags: [],
  reactions: undefined,
};

const summary = {
  threadHeadId: message.id,
  replyCount: 2,
  lastReplyAt: null,
  participants: [],
};

test("inline toggle is independent from the existing thread opener", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { MessageThreadSummaryRow } = await import(
    "./MessageThreadSummaryRow.tsx"
  );
  let openCount = 0;
  let toggleCount = 0;
  const renderSummary = (inlineExpanded) =>
    React.createElement(MessageThreadSummaryRow, {
      inlineExpanded,
      message,
      onOpenThread: () => {
        openCount += 1;
      },
      onToggleInline: () => {
        toggleCount += 1;
      },
      showDepthGuides: false,
      summary,
    });

  const view = render(renderSummary(false));
  const inlineToggle = view.getByRole("button", {
    name: "Show 2 replies in channel",
  });
  assert.equal(inlineToggle.getAttribute("aria-pressed"), "false");
  fireEvent.click(inlineToggle);
  assert.equal(toggleCount, 1);
  assert.equal(openCount, 0);

  view.rerender(renderSummary(true));
  const hideToggle = view.getByRole("button", {
    name: "Hide 2 replies from channel",
  });
  assert.equal(hideToggle.getAttribute("aria-pressed"), "true");
  assert.equal(hideToggle.textContent, "Hide replies");

  fireEvent.click(
    view.getByRole("button", { name: "View thread with 2 replies" }),
  );
  assert.equal(openCount, 1);
  assert.equal(toggleCount, 1);
});
