/**
 * An acknowledged send must stay in the frozen unread tail when its event
 * comes back without the optimistic `localKey`, from a head refetch or from a
 * live echo after the ack. Only arrivals newer than the snapshot are pending.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { cleanup, renderHook } from "@testing-library/react";

import { createOptimisticMessage } from "../hooks.ts";
import { reconcileChannelWindowMessages } from "../lib/channelWindowReconciliation.ts";
import {
  emptyChannelWindowStore,
  mergeLiveChannelWindowEvent,
  replaceNewestChannelWindow,
} from "../lib/channelWindowStore.ts";
import { formatTimelineMessages } from "../lib/formatTimelineMessages.ts";
import { useBufferedTimelineMessages } from "./useBufferedTimelineMessages.ts";

afterEach(cleanup);

const channelId = "channel";
const pubkey = "a".repeat(64);

function relayEvent(id, createdAt, content = id) {
  return {
    id: id.padEnd(64, "0"),
    pubkey,
    created_at: createdAt,
    kind: 9,
    tags: [["h", channelId]],
    content,
    sig: "b".repeat(128),
  };
}

function newestPage(events) {
  return {
    startCursor: null,
    rows: events.map((event) => ({ event, thread: null })),
    aux: [],
    nextCursor: null,
    hasMore: false,
  };
}

const cases = [
  {
    path: "a head refetch",
    returnWithoutLocalKey: (window, accepted, earlier) =>
      replaceNewestChannelWindow(window, newestPage([accepted, earlier])),
  },
  {
    path: "a live echo after the ack",
    returnWithoutLocalKey: (window, accepted) =>
      mergeLiveChannelWindowEvent(window, accepted),
  },
];

for (const { path, returnWithoutLocalKey } of cases) {
  test(`an acknowledged send stays frozen when ${path} drops its localKey`, () => {
    const now = Math.floor(Date.now() / 1_000);
    const earlier = relayEvent("earlier", now - 60);
    let window = replaceNewestChannelWindow(
      emptyChannelWindowStore(),
      newestPage([earlier]),
    );
    let messages = reconcileChannelWindowMessages(window, []);
    const advance = (next) => {
      window = next;
      messages = reconcileChannelWindowMessages(window, messages);
      return formatTimelineMessages(messages, null, pubkey, null);
    };

    // Send, then acknowledge the way useSendMessageMutation does.
    const optimistic = createOptimisticMessage(
      channelId,
      "hello",
      { pubkey },
      messages,
    );
    advance(mergeLiveChannelWindowEvent(window, optimistic));
    const accepted = {
      ...relayEvent("accepted", optimistic.created_at, optimistic.content),
      tags: optimistic.tags,
    };
    const acknowledged = advance(
      mergeLiveChannelWindowEvent(
        {
          ...window,
          liveOverlay: window.liveOverlay.filter(
            (event) => event.id !== optimistic.id,
          ),
        },
        { ...accepted, localKey: optimistic.id },
      ),
    );

    // The reader scrolls up, freezing the tail with the acknowledged row.
    const { result, rerender } = renderHook(
      (props) => useBufferedTimelineMessages({ channelId, ...props }),
      { initialProps: { isAtBottom: true, messages: acknowledged } },
    );
    rerender({ isAtBottom: false, messages: acknowledged });

    advance(returnWithoutLocalKey(window, accepted, earlier));
    const newer = relayEvent("newer", optimistic.created_at + 1);
    rerender({
      isAtBottom: false,
      messages: advance(mergeLiveChannelWindowEvent(window, newer)),
    });

    assert.deepEqual(
      result.current.messages.map(({ id }) => id),
      [earlier.id, accepted.id],
    );
    assert.equal(result.current.pendingCount, 1);
  });
}
