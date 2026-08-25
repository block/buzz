import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>");
globalThis.window = dom.window;
globalThis.document = dom.window.document;
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
globalThis.HTMLElement = dom.window.HTMLElement;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const React = await import("react");
const { act } = React;
const { createRoot } = await import("react-dom/client");
const { useChannelRouteTarget } = await import("./useChannelRouteTarget.ts");

const target = {
  id: "target",
  author: "alice",
  body: "hello",
  createdAt: 1,
  depth: 0,
  parentId: null,
  rootId: null,
  tags: [],
  time: "now",
};

function Harness({ calls, threadRootId }) {
  useChannelRouteTarget({
    activeChannel: { id: "channel", channelType: "stream" },
    activeChannelId: "channel",
    closeAgentSession: () => calls.push("close-agent"),
    requireThreadEditResolution: () => true,
    setEditTargetId: () => {},
    setExpandedThreadReplyIds: () => {},
    setOpenThreadHeadId: (id) => calls.push(`open:${id}`),
    setProfilePanelPubkey: () => {},
    setThreadReplyTargetId: () => {},
    setThreadScrollTargetId: () => {},
    targetMessageId: "target",
    targetThreadRootId: threadRootId,
    timelineMessages: [target],
  });
  return null;
}

test("the same top-level target can advance from timeline-only to open-thread", async () => {
  const calls = [];
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(React.createElement(Harness, { calls, threadRootId: null }));
  });
  assert.deepEqual(calls, []);

  await act(async () => {
    root.render(
      React.createElement(Harness, { calls, threadRootId: "target" }),
    );
  });
  assert.deepEqual(calls, ["close-agent", "open:target"]);
  await act(async () => root.unmount());
});
