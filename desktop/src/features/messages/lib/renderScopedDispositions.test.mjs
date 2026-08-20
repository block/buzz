import assert from "node:assert/strict";
import test from "node:test";

import {
  claimUnhydratedRenderScopedDispositionIds,
  hydrateRenderScopedDispositions,
  releaseRenderScopedDispositionIds,
  resetRenderScopedDispositionHydration,
} from "./renderScopedDispositions.ts";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { channelMessagesKey } from "./messageQueryKeys.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";

function hex(char) {
  return char.repeat(64);
}

function event(id, kind, overrides = {}) {
  return {
    id,
    pubkey: hex("a"),
    kind,
    created_at: 1_700_000_000,
    content: "",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function makeQueryClientStub(initialEvents = []) {
  const store = new Map([
    [JSON.stringify(channelMessagesKey(CHANNEL_ID)), initialEvents],
  ]);
  return {
    getQueryData(key) {
      return store.get(JSON.stringify(key));
    },
    setQueryData(key, updater) {
      const k = JSON.stringify(key);
      const next =
        typeof updater === "function" ? updater(store.get(k) ?? []) : updater;
      store.set(k, next);
      return next;
    },
  };
}

test.afterEach(() => {
  resetRenderScopedDispositionHydration();
});

test("claims each rendered message id once per channel and can retry released ids", () => {
  assert.deepEqual(
    claimUnhydratedRenderScopedDispositionIds(CHANNEL_ID, [
      hex("1"),
      hex("2"),
      hex("1"),
    ]),
    [hex("1"), hex("2")],
  );
  assert.deepEqual(
    claimUnhydratedRenderScopedDispositionIds(CHANNEL_ID, [hex("1"), hex("2")]),
    [],
  );
  assert.deepEqual(
    claimUnhydratedRenderScopedDispositionIds("other-channel", [hex("1")]),
    [hex("1")],
  );

  releaseRenderScopedDispositionIds(CHANNEL_ID, [hex("2")]);
  assert.deepEqual(
    claimUnhydratedRenderScopedDispositionIds(CHANNEL_ID, [hex("1"), hex("2")]),
    [hex("2")],
  );
});

test("hydrates visible dispositions into the channel timeline cache", async () => {
  const messageId = hex("1");
  const dispositionId = hex("2");
  const message = event(messageId, 9, {
    pubkey: hex("a"),
    content: "@agent do the thing",
    // Marked as a request targeting hex("b") — the disposition below is
    // signed by hex("b"), and the binding check requires the request to
    // name its signer as an `agent` target (see formatTimelineMessages.ts).
    tags: [
      ["h", CHANNEL_ID],
      ["t", "request"],
      ["agent", hex("b")],
      ["p", hex("b")],
    ],
  });
  const disposition = event(dispositionId, 44300, {
    pubkey: hex("b"),
    content: JSON.stringify({ disposition: "completed", reason: "" }),
    tags: [
      ["h", CHANNEL_ID],
      ["e", messageId],
      ["p", hex("a")],
      ["disposition", "completed"],
    ],
  });
  const queryClient = makeQueryClientStub([message]);
  const calls = [];

  await hydrateRenderScopedDispositions({
    channelId: CHANNEL_ID,
    messageIds: [messageId],
    queryClient,
    deps: {
      fetchDispositionEventsForMessages: async (channelId, messageIds) => {
        calls.push({ channelId, messageIds });
        return [disposition];
      },
    },
  });

  assert.deepEqual(calls, [{ channelId: CHANNEL_ID, messageIds: [messageId] }]);
  const cached = queryClient.getQueryData(channelMessagesKey(CHANNEL_ID));
  assert.ok(cached.some((e) => e.id === dispositionId));

  const timeline = formatTimelineMessages(cached, null, undefined, null);
  assert.deepEqual(timeline.find((m) => m.id === messageId)?.requestStatus, {
    kind: "valid",
    outcome: { kind: "settled", state: "completed" },
    latestObservation: "completed",
    reason: "",
    warnings: [],
    resolved: true,
  });
});

test("failed hydration releases ids so the next render can retry", async () => {
  const messageId = hex("1");
  const queryClient = makeQueryClientStub([event(messageId, 9)]);

  await hydrateRenderScopedDispositions({
    channelId: CHANNEL_ID,
    messageIds: [messageId],
    queryClient,
    deps: {
      fetchDispositionEventsForMessages: async () => {
        throw new Error("relay timeout");
      },
    },
  });

  assert.deepEqual(
    claimUnhydratedRenderScopedDispositionIds(CHANNEL_ID, [messageId]),
    [messageId],
  );
});
