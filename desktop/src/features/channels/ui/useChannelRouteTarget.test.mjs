import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});
after(() => dom.window.close());

test("timeline intent keeps an ordinary reply out of the thread panel", async () => {
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useChannelRouteTarget } = await import("./useChannelRouteTarget.ts");
  const calls = [];
  const root = {
    id: "root",
    parentId: null,
    rootId: null,
    tags: [],
  };
  const reply = {
    id: "reply",
    parentId: "root",
    rootId: "root",
    tags: [
      ["e", "root", "", "root"],
      ["e", "root", "", "reply"],
    ],
  };

  const hook = renderHook(() =>
    useChannelRouteTarget({
      activeChannel: { id: "channel", channelType: "stream" },
      activeChannelId: "channel",
      closeAgentSession: () => calls.push("close-agent"),
      requireThreadEditResolution: () => {
        calls.push("resolve-edit");
        return true;
      },
      setEditTargetId: () => calls.push("edit"),
      setExpandedThreadReplyIds: () => calls.push("expand"),
      setOpenThreadHeadId: () => calls.push("open-thread"),
      setProfilePanelPubkey: () => calls.push("profile"),
      setThreadReplyTargetId: () => calls.push("reply-target"),
      setThreadScrollTargetId: () => calls.push("scroll"),
      targetMessageId: "reply",
      targetMessageView: "timeline",
      timelineMessages: [root, reply],
    }),
  );

  assert.equal(hook.result.current, "root");
  assert.deepEqual(calls, []);
  hook.unmount();
  cleanup();
});

test("timeline intent preserves the exact id of a broadcast reply row", async () => {
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useChannelRouteTarget } = await import("./useChannelRouteTarget.ts");
  const broadcast = {
    id: "broadcast-reply",
    parentId: "root",
    rootId: "root",
    tags: [
      ["e", "root", "", "root"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ],
  };

  const hook = renderHook(() =>
    useChannelRouteTarget({
      activeChannel: { id: "channel", channelType: "stream" },
      activeChannelId: "channel",
      closeAgentSession: () => {},
      requireThreadEditResolution: () => true,
      setEditTargetId: () => {},
      setExpandedThreadReplyIds: () => {},
      setOpenThreadHeadId: () => {},
      setProfilePanelPubkey: () => {},
      setThreadReplyTargetId: () => {},
      setThreadScrollTargetId: () => {},
      targetMessageId: "broadcast-reply",
      targetMessageView: "timeline",
      timelineMessages: [broadcast],
    }),
  );

  assert.equal(hook.result.current, "broadcast-reply");
  hook.unmount();
  cleanup();
});

test("different view intents are handled independently for the same target", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useChannelRouteTarget } = await import("./useChannelRouteTarget.ts");
  const calls = [];
  const root = {
    id: "root",
    parentId: null,
    rootId: null,
    tags: [],
  };

  const hook = renderHook(
    ({ targetMessageView }) =>
      useChannelRouteTarget({
        activeChannel: { id: "channel", channelType: "stream" },
        activeChannelId: "channel",
        closeAgentSession: () => {},
        requireThreadEditResolution: () => true,
        setEditTargetId: () => {},
        setExpandedThreadReplyIds: () => {},
        setOpenThreadHeadId: () => calls.push("open-thread"),
        setProfilePanelPubkey: () => {},
        setThreadReplyTargetId: () => {},
        setThreadScrollTargetId: () => {},
        targetMessageId: "root",
        targetMessageView,
        timelineMessages: [root],
      }),
    { initialProps: { targetMessageView: undefined } },
  );

  assert.deepEqual(calls, ["open-thread"]);
  await act(async () => {
    hook.rerender({ targetMessageView: "timeline" });
  });
  assert.deepEqual(calls, ["open-thread"]);
  hook.unmount();
  cleanup();
});
