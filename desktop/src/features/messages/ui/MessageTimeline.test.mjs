import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, afterEach, test } from "node:test";

import { JSDOM } from "jsdom";
import * as React from "react";

// Drive the virtualizer's public bottom-state callback deterministically; jsdom
// has no layout. Keep MessageTimeline, its buffer/scroll hooks and UnreadPill
// real, so this covers the control's consumption of semantic AND physical state.
const hooks = registerHooks({
  resolve(specifier, context, nextResolve) {
    if (
      specifier === "./TimelineMessageList" &&
      context.parentURL?.endsWith("/MessageTimeline.tsx")
    ) {
      return { shortCircuit: true, url: "buzz-timeline-test:list" };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-timeline-test:list") {
      return {
        format: "module",
        shortCircuit: true,
        source:
          "export function TimelineMessageList(props) { return globalThis.__TIMELINE_TEST_LIST__(props); }",
      };
    }
    return nextLoad(url, context);
  },
});

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
  pretendToBeVisual: true,
});
Object.assign(globalThis, {
  document: dom.window.document,
  window: dom.window,
  HTMLElement: dom.window.HTMLElement,
  localStorage: dom.window.localStorage,
  IS_REACT_ACT_ENVIRONMENT: true,
  requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
  cancelAnimationFrame: dom.window.cancelAnimationFrame.bind(dom.window),
});

const { act, cleanup, fireEvent, render } = await import(
  "@testing-library/react"
);
const { MessageTimeline } = await import("./MessageTimeline.tsx");
let listProps;
let bottomJumps = [];
const virtualizerApi = {
  scrollToBottom: (behavior) => bottomJumps.push(behavior),
  settleAtBottom: () => {},
  cancelBottomIntent: () => {},
  scrollToMessage: () => false,
};
globalThis.__TIMELINE_TEST_LIST__ = function TimelineList(props) {
  listProps = props;
  React.useEffect(() => {
    props.onVirtualizerApiChange(virtualizerApi);
  }, [props.onVirtualizerApiChange]);
  return React.createElement(
    "div",
    { "data-testid": "test-message-list" },
    props.messages.map((message) =>
      React.createElement("p", { key: message.id }, message.body),
    ),
  );
};

afterEach(() => {
  cleanup();
  bottomJumps = [];
});
after(() => {
  hooks.deregister();
  delete globalThis.__TIMELINE_TEST_LIST__;
  dom.window.close();
});

const row = (id, body) => ({
  id,
  body,
  author: "Alice",
  createdAt: 1,
  time: "12:00",
  depth: 0,
});
const initial = row("initial", "Already reading");
const incoming = row("incoming", "Accepted shared message");
const older = row("older", "Earlier history");

async function frame() {
  await act(async () => {
    await new Promise((resolve) => window.requestAnimationFrame(resolve));
  });
}

async function reportBottom(atBottom) {
  act(() => listProps.onAtBottomStateChange(atBottom));
  await frame();
}

function mount(messages = [initial]) {
  return render(
    React.createElement(MessageTimeline, { channelId: "channel-a", messages }),
  );
}

function update(view, messages) {
  view.rerender(
    React.createElement(MessageTimeline, { channelId: "channel-a", messages }),
  );
}

test("buffered arrivals keep a clickable catch-up at physical bottom without releasing the semantic tail", async () => {
  const view = mount();
  await reportBottom(true);
  await reportBottom(false);
  // A frozen-model reflow reports physical bottom; MessageTimeline suppresses
  // this synthetic transition, preserving the reader's semantic freeze.
  await reportBottom(true);
  bottomJumps = [];
  assert.equal(view.queryByTestId("message-scroll-to-latest"), null);

  update(view, [initial, incoming]);
  assert.equal(view.queryByText(incoming.body), null);
  const catchUp = view.getByRole("button", { name: "1 new message" });
  assert.equal(catchUp.dataset.testid, "message-scroll-to-latest");
  assert.equal(catchUp.disabled, false);
  assert.deepEqual(bottomJumps, [], "arrival must not force a scroll");

  fireEvent.click(catchUp);
  await frame();
  assert.ok(view.getByText(incoming.body));
  assert.equal(view.queryByTestId("message-scroll-to-latest"), null);
  assert.deepEqual(
    bottomJumps,
    ["auto"],
    "explicit catch-up keeps its existing scroll action",
  );
});

test("history browsing keeps Jump to latest and admits prepends without releasing new arrivals", async () => {
  const view = mount();
  await reportBottom(true);
  await reportBottom(false);
  assert.ok(view.getByRole("button", { name: "Jump to latest" }));
  bottomJumps = [];

  update(view, [older, initial, incoming]);
  // Wait for the real prepend settle gate (quiet window plus stable frames).
  assert.ok(await view.findByText(older.body));
  assert.ok(view.getByText(initial.body));
  assert.equal(view.queryByText(incoming.body), null);
  assert.ok(view.getByRole("button", { name: "1 new message" }));
  assert.deepEqual(bottomJumps, []);
});

test("a real return to the semantic tail still releases buffered arrivals", async () => {
  const view = mount();
  await reportBottom(true);
  await reportBottom(false);
  await reportBottom(true); // suppressed synthetic return
  update(view, [initial, incoming]);
  assert.equal(view.queryByText(incoming.body), null);

  await reportBottom(false);
  await reportBottom(true); // genuine reader return
  assert.ok(view.getByText(incoming.body));
  assert.equal(view.queryByTestId("message-scroll-to-latest"), null);
});

test("the live tail with no pending rows does not invent a catch-up pill", async () => {
  const view = mount();
  await reportBottom(true);
  assert.equal(view.queryByTestId("message-scroll-to-latest"), null);
  update(view, [initial, incoming]);
  assert.ok(view.getByText(incoming.body));
  assert.equal(view.queryByTestId("message-scroll-to-latest"), null);
});
