import assert from "node:assert/strict";
import test from "node:test";

import {
  replayLiveSubscriptions,
  replayReconnectHistoryPages,
} from "@/shared/api/relayReconnectReplay";
import { buildChannelFilter } from "@/shared/api/relayChannelFilters";
import {
  handleRelayClosed,
  handleSubscriptionEose,
} from "@/shared/api/relayClosedRecovery";
import {
  shouldDeliverThreadReplyDesktopNotification,
  ThreadReplyNotificationDedupe,
  THREAD_REPLY_SEEN_MAX_ITEMS,
} from "./threadReplyNotificationDedupe.ts";
import {
  disposeStaleLiveChannelSubscriptions,
  reconcileAfterLiveSubscriptionAdditions,
} from "./useLiveChannelUpdates.ts";

function memoryStorage(initialRecords = []) {
  const values = new Map();
  let seeded = false;
  let writeCount = 0;
  return {
    getItem: (key) => {
      if (!seeded && initialRecords.length > 0) {
        values.set(key, JSON.stringify(initialRecords));
        seeded = true;
      }
      return values.get(key) ?? null;
    },
    setItem: (key, value) => {
      writeCount += 1;
      values.set(key, value);
    },
    get writeCount() {
      return writeCount;
    },
  };
}

function deferred() {
  let resolve;
  const promise = new Promise((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

test("reconnect prunes before newest-first replay and retains the boundary ID", async () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());
  const notified = [];

  dedupe.setChannelReplayFloor("channel", 900);
  assert.equal(dedupe.record("channel", "boundary", 996), true);

  // The original live filter starts at 900, so the 996 boundary event remains
  // inside the history window that the relay can restore.
  dedupe.handleConnectionState("reconnecting");

  const newestPage = Array.from({ length: 500 }, (_, index) => ({
    id: `offline-${index}`,
    created_at: 1_501 + index,
  }));
  const pages = [newestPage, [{ id: "boundary", created_at: 996 }]];
  await replayReconnectHistoryPages({
    subscription: {
      mode: "live",
      filter: { kinds: [], limit: 500 },
      onEvent: (event) => {
        if (dedupe.record("channel", event.id, event.created_at)) {
          notified.push(event.id);
        }
      },
    },
    since: 995,
    until: 2_000,
    isActive: () => true,
    requestHistory: async () => pages.shift() ?? [],
  });

  assert.equal(notified.length, 500);
  assert.equal(notified.includes("offline-0"), true);
  assert.equal(notified.includes("boundary"), false);
  assert.equal(
    dedupe.record("channel", "boundary", 996),
    false,
    "newest replay delivery must not evict the older boundary event",
  );
});

test("offline arrival notifies once and is persisted across remount", () => {
  const storage = memoryStorage();
  const first = new ThreadReplyNotificationDedupe("user", storage);

  first.setChannelReplayFloor("channel", 100);
  first.handleConnectionState("reconnecting");

  assert.equal(first.record("channel", "offline", 200), true);
  assert.equal(first.record("channel", "offline", 200), false);
  first.flush();

  const remounted = new ThreadReplyNotificationDedupe("user", storage);
  remounted.setChannelReplayFloor("channel", 100);
  remounted.handleConnectionState("reconnecting");
  assert.equal(remounted.record("channel", "offline", 200), false);
});

test("late restored-live duplicate stays suppressed until EOSE completes", async () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());
  const notified = [];
  const onEvent = (event) => {
    if (dedupe.record("channel", event.id, event.created_at)) {
      notified.push(event.id);
    }
  };
  const subscription = {
    mode: "live",
    filter: {
      ...buildChannelFilter("channel", 1_000),
      since: 900,
    },
    onEvent,
    lastSeenCreatedAt: 1_000,
  };
  const subscriptions = new Map([["live", subscription]]);

  dedupe.setChannelReplayFloor("channel", 900);
  assert.equal(dedupe.record("channel", "boundary", 996), true);
  dedupe.handleConnectionState("reconnecting");

  const replayPromise = replayLiveSubscriptions({
    subscriptions,
    now: 2_000,
    sendRaw: async () => {},
    requestHistory: async () => [{ id: "offline", created_at: 2_000 }],
  });
  await new Promise((resolve) => setImmediate(resolve));

  // The restored live REQ can deliver its duplicate after HTTP paging. EOSE
  // is the barrier that must keep the connection in replay mode until this
  // callback has been handed to the client event buffer.
  onEvent({ id: "boundary", created_at: 996 });
  subscription.resolveReconnectEose();
  await replayPromise;
  dedupe.handleConnectionState("connected");

  assert.deepEqual(notified, ["offline"]);
});

test("active-channel reply is recorded before desktop suppression", () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());

  dedupe.setChannelReplayFloor("channel", 100);
  const firstSeen = dedupe.record("channel", "active-reply", 100);
  assert.equal(
    shouldDeliverThreadReplyDesktopNotification({
      isFirstSeen: firstSeen,
      isEligible: true,
      isActiveChannel: true,
      notifyForActiveChannel: false,
    }),
    false,
  );

  dedupe.handleConnectionState("reconnecting");
  const replayFirstSeen = dedupe.record("channel", "active-reply", 100);
  assert.equal(
    shouldDeliverThreadReplyDesktopNotification({
      isFirstSeen: replayFirstSeen,
      isEligible: true,
      isActiveChannel: false,
      notifyForActiveChannel: false,
    }),
    false,
    "navigating away before reconnect must not surface a stale notification",
  );
});

test("reply observed while ineligible cannot notify after eligibility changes", () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());
  dedupe.reconcileActiveChannels(["channel"]);
  dedupe.setChannelReplayFloor("channel", 100);

  const firstSeen = dedupe.record("channel", "muted-reply", 100);
  assert.equal(
    shouldDeliverThreadReplyDesktopNotification({
      isFirstSeen: firstSeen,
      isEligible: false,
      isActiveChannel: false,
      notifyForActiveChannel: false,
    }),
    false,
  );

  dedupe.handleConnectionState("reconnecting");
  const replayFirstSeen = dedupe.record("channel", "muted-reply", 100);
  assert.equal(
    shouldDeliverThreadReplyDesktopNotification({
      isFirstSeen: replayFirstSeen,
      isEligible: true,
      isActiveChannel: false,
      notifyForActiveChannel: false,
    }),
    false,
    "unmute/follow changes must not turn a replay into a stale notification",
  );
});

test("startup reconciliation removes departed records before cap eviction", () => {
  const activeRecord = {
    eventId: "active-boundary",
    channelId: "active",
    createdAt: 1,
  };
  const departedRecords = Array.from(
    { length: THREAD_REPLY_SEEN_MAX_ITEMS },
    (_, index) => ({
      eventId: `departed-${index}`,
      channelId: "departed",
      createdAt: index + 2,
    }),
  );
  const dedupe = new ThreadReplyNotificationDedupe(
    "user",
    memoryStorage([activeRecord, ...departedRecords]),
  );

  dedupe.reconcileActiveChannels(["active"]);
  dedupe.setChannelReplayFloor("active", 0);

  assert.equal(
    dedupe.record("active", "active-boundary", 1),
    false,
    "departed records must be removed before they can evict an active ID",
  );
  assert.equal(dedupe.record("departed", "departed-0", 2), false);
  dedupe.reconcileActiveChannels(["active", "departed"]);
  dedupe.setChannelReplayFloor("departed", 0);
  assert.equal(dedupe.record("departed", "departed-0", 2), true);
});

test("startup provisional target retains its persisted same-second boundary", () => {
  const boundary = {
    eventId: "persisted-boundary",
    channelId: "target",
    createdAt: 100,
  };
  const dedupe = new ThreadReplyNotificationDedupe(
    "user",
    memoryStorage([boundary]),
  );

  dedupe.reconcileActiveChannels(["target"]);
  dedupe.setChannelReplayFloor("target", 100);

  assert.equal(
    dedupe.record("target", boundary.eventId, boundary.createdAt),
    false,
    "a target channel must stay persisted while its subscription settles",
  );
});

test("channel removal deletes its replay floor and persisted records", () => {
  const storage = memoryStorage();
  const dedupe = new ThreadReplyNotificationDedupe("user", storage);
  dedupe.reconcileActiveChannels(["active", "departed"]);
  dedupe.setChannelReplayFloor("active", 0);
  dedupe.setChannelReplayFloor("departed", 0);
  assert.equal(dedupe.record("active", "active-reply", 1), true);
  assert.equal(dedupe.record("departed", "departed-reply", 2), true);

  dedupe.reconcileActiveChannels(["active"]);

  const remounted = new ThreadReplyNotificationDedupe("user", storage);
  remounted.reconcileActiveChannels(["active"]);
  remounted.setChannelReplayFloor("active", 0);
  assert.equal(remounted.record("active", "active-reply", 1), false);
  remounted.reconcileActiveChannels(["active", "departed"]);
  remounted.setChannelReplayFloor("departed", 0);
  assert.equal(remounted.record("departed", "departed-reply", 2), true);
});

test("canceled sync cannot overwrite replacement reconciliation", async () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());
  const additionA = deferred();
  const additionB = deferred();
  let currentGeneration = 1;
  let syncACancelled = false;

  const syncA = reconcileAfterLiveSubscriptionAdditions({
    additions: [additionA.promise],
    isCurrentSync: () => !syncACancelled && currentGeneration === 1,
    reconcile: () => dedupe.reconcileActiveChannels([]),
  });

  syncACancelled = true;
  currentGeneration = 2;
  dedupe.reconcileActiveChannels([]);
  dedupe.setChannelReplayFloor("replacement", 100);
  const syncB = reconcileAfterLiveSubscriptionAdditions({
    additions: [additionB.promise],
    isCurrentSync: () => currentGeneration === 2,
    reconcile: () => dedupe.reconcileActiveChannels(["replacement"]),
  });

  additionA.resolve();
  assert.equal(await syncA, false);
  assert.equal(
    dedupe.record("replacement", "live-reply", 100),
    true,
    "a stale sync must not reject an event delivered before sync B resolves",
  );

  additionB.resolve();
  assert.equal(await syncB, true);
  dedupe.handleConnectionState("reconnecting");
  assert.equal(
    dedupe.record("replacement", "live-reply", 100),
    false,
    "the live sighting must stay recorded when the same event replays",
  );
});

test("superseding pending sync provisionally retains the target boundary", async () => {
  const boundary = {
    eventId: "pending-boundary",
    channelId: "replacement",
    createdAt: 100,
  };
  const dedupe = new ThreadReplyNotificationDedupe(
    "user",
    memoryStorage([boundary]),
  );
  const addition = deferred();

  dedupe.reconcileActiveChannels(["replacement"]);
  dedupe.setChannelReplayFloor("replacement", 100);
  const sync = reconcileAfterLiveSubscriptionAdditions({
    additions: [addition.promise],
    isCurrentSync: () => true,
    reconcile: () => dedupe.reconcileActiveChannels(["replacement"]),
  });

  assert.equal(
    dedupe.record("replacement", boundary.eventId, boundary.createdAt),
    false,
  );
  addition.resolve();
  assert.equal(await sync, true);
});

test("identity handoff disposes old recovery ownership before recreating", () => {
  const oldStorage = memoryStorage();
  const nextStorage = memoryStorage();
  const oldDedupe = new ThreadReplyNotificationDedupe("old", oldStorage);
  const nextDedupe = new ThreadReplyNotificationDedupe("next", nextStorage);
  oldDedupe.reconcileActiveChannels(["channel"]);
  oldDedupe.handleSubscriptionRecoveryState(true);
  const oldWritesDuringRecovery = oldStorage.writeCount;
  let disposeCalls = 0;
  const activeSubs = new Map([
    [
      "channel",
      {
        replayFloor: 100,
        dedupe: oldDedupe,
        dispose: async () => {
          disposeCalls += 1;
          oldDedupe.handleSubscriptionRecoveryState(false);
        },
      },
    ],
  ]);

  disposeStaleLiveChannelSubscriptions({
    activeSubs,
    targetIds: new Set(["channel"]),
    dedupe: nextDedupe,
  });
  assert.equal(disposeCalls, 1);
  assert.equal(activeSubs.size, 0);
  assert.equal(oldStorage.writeCount, oldWritesDuringRecovery + 1);

  nextDedupe.reconcileActiveChannels(["channel"]);
  nextDedupe.handleSubscriptionRecoveryState(true);
  const nextWritesDuringRecovery = nextStorage.writeCount;
  assert.equal(nextDedupe.record("channel", "next-replay", 100), true);
  assert.equal(nextStorage.writeCount, nextWritesDuringRecovery);
  nextDedupe.handleSubscriptionRecoveryState(false);
  assert.equal(nextStorage.writeCount, nextWritesDuringRecovery + 1);
});

test("full-cap newest-first CLOSED recovery retains the older boundary ID", () => {
  const originalWindow = globalThis.window;
  const scheduled = [];
  globalThis.window = {
    clearTimeout: () => {},
    setTimeout: (callback, delay) => {
      scheduled.push({ callback, delay });
      return scheduled.length;
    },
  };
  try {
    const dedupe = new ThreadReplyNotificationDedupe("user", undefined);
    dedupe.reconcileActiveChannels(["channel"]);
    dedupe.setChannelReplayFloor("channel", 0);
    assert.equal(dedupe.record("channel", "boundary", 1), true);
    for (let index = 1; index < THREAD_REPLY_SEEN_MAX_ITEMS; index += 1) {
      assert.equal(dedupe.record("channel", `seen-${index}`, index + 1), true);
    }

    const subscription = {
      mode: "live",
      filter: { kinds: [9], "#h": ["channel"], limit: 1_000, since: 0 },
      onEvent: () => {},
      onClosedRecoveryStateChange: (recovering) =>
        dedupe.handleSubscriptionRecoveryState(recovering),
    };
    const subscriptions = new Map([["live-1", subscription]]);
    handleRelayClosed({
      subscriptions,
      subId: "live-1",
      message: "error: database unavailable",
      sendReq: async () => {},
    });

    assert.deepEqual(
      scheduled.map(({ delay }) => delay).sort((left, right) => left - right),
      [1_000, 8_000],
    );
    assert.equal(dedupe.record("channel", "newest-replay", 2_001), true);
    assert.equal(
      dedupe.record("channel", "boundary", 1),
      false,
      "cap eviction must remain suspended until the older replay page arrives",
    );

    handleSubscriptionEose({
      subscriptions,
      subId: "live-1",
      closeSubscription: async () => {},
    });
  } finally {
    globalThis.window = originalWindow;
  }
});

test("overlapping socket and CLOSED recovery flush only after both complete", () => {
  const storage = memoryStorage();
  const dedupe = new ThreadReplyNotificationDedupe("user", storage);
  dedupe.reconcileActiveChannels(["channel"]);
  const writesBeforeReplay = storage.writeCount;

  dedupe.handleSubscriptionRecoveryState(true);
  dedupe.handleConnectionState("reconnecting");
  assert.equal(dedupe.record("channel", "offline", 100), true);
  dedupe.handleSubscriptionRecoveryState(false);
  assert.equal(storage.writeCount, writesBeforeReplay);

  dedupe.handleConnectionState("connected");
  assert.equal(storage.writeCount, writesBeforeReplay + 1);
});

test("stalled socket acquires replay ownership before CLOSED recovery ends", () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", undefined);
  dedupe.reconcileActiveChannels(["channel"]);
  dedupe.setChannelReplayFloor("channel", 0);
  assert.equal(dedupe.record("channel", "boundary", 1), true);
  for (let index = 1; index < THREAD_REPLY_SEEN_MAX_ITEMS; index += 1) {
    assert.equal(dedupe.record("channel", `seen-${index}`, index + 1), true);
  }

  dedupe.handleSubscriptionRecoveryState(true);
  assert.equal(dedupe.record("channel", "newest-replay", 2_001), true);
  dedupe.handleConnectionState("stalled");
  dedupe.handleSubscriptionRecoveryState(false);
  assert.equal(
    dedupe.record("channel", "boundary", 1),
    false,
    "stalled reset must retain the boundary until socket replay completes",
  );
  dedupe.handleConnectionState("reconnecting");
  dedupe.handleConnectionState("connected");
});

test("pruning is per-channel and follows each live filter's actual since", () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());

  dedupe.reconcileActiveChannels(["busy", "quiet"]);
  dedupe.setChannelReplayFloor("busy", 900);
  dedupe.setChannelReplayFloor("quiet", 0);
  assert.equal(dedupe.record("busy", "busy-old", 994), true);
  assert.equal(dedupe.record("busy", "busy-floor", 995), true);
  assert.equal(dedupe.record("quiet", "quiet-boundary", 96), true);

  dedupe.setChannelReplayFloor("busy", 995);
  dedupe.setChannelReplayFloor("quiet", 95);

  assert.equal(dedupe.record("busy", "busy-old", 994), true);
  assert.equal(dedupe.record("busy", "busy-floor", 995), false);
  assert.equal(
    dedupe.record("quiet", "quiet-boundary", 96),
    false,
    "busy-channel replay floor must not evict quiet-channel records",
  );
});

test("size cap evicts oldest records only after replay completes", () => {
  const dedupe = new ThreadReplyNotificationDedupe("user", memoryStorage());

  dedupe.reconcileActiveChannels(["channel"]);
  dedupe.setChannelReplayFloor("channel", 0);
  dedupe.handleConnectionState("reconnecting");
  for (let index = 0; index <= THREAD_REPLY_SEEN_MAX_ITEMS; index += 1) {
    assert.equal(dedupe.record("channel", `event-${index}`, index + 1), true);
  }

  assert.equal(
    dedupe.record("channel", "event-0", 1),
    false,
    "cap must remain suspended while older replay pages can still arrive",
  );

  dedupe.handleConnectionState("connected");
  assert.equal(dedupe.record("channel", "event-0", 1), true);
  assert.equal(
    dedupe.record(
      "channel",
      `event-${THREAD_REPLY_SEEN_MAX_ITEMS}`,
      THREAD_REPLY_SEEN_MAX_ITEMS + 1,
    ),
    false,
    "newest records must survive the cap",
  );
});

test("replay coalesces persistence and stores the final capped set", () => {
  const storage = memoryStorage();
  const dedupe = new ThreadReplyNotificationDedupe("user", storage);

  dedupe.reconcileActiveChannels(["channel"]);
  dedupe.setChannelReplayFloor("channel", 0);
  dedupe.handleConnectionState("reconnecting");
  const writesBeforeReplay = storage.writeCount;

  for (let index = 0; index <= THREAD_REPLY_SEEN_MAX_ITEMS; index += 1) {
    assert.equal(dedupe.record("channel", `replay-${index}`, index + 1), true);
  }

  assert.equal(
    storage.writeCount,
    writesBeforeReplay,
    "replayed events must not synchronously rewrite the growing cache",
  );

  dedupe.handleConnectionState("connected");
  assert.equal(storage.writeCount, writesBeforeReplay + 1);

  const remounted = new ThreadReplyNotificationDedupe("user", storage);
  remounted.reconcileActiveChannels(["channel"]);
  remounted.setChannelReplayFloor("channel", 0);
  assert.equal(remounted.record("channel", "replay-0", 1), true);
  assert.equal(
    remounted.record(
      "channel",
      `replay-${THREAD_REPLY_SEEN_MAX_ITEMS}`,
      THREAD_REPLY_SEEN_MAX_ITEMS + 1,
    ),
    false,
    "the single replay flush must persist the capped newest records",
  );
});

test("storage failure disables repeated serialization attempts", () => {
  let writeCount = 0;
  const storage = {
    getItem: () => null,
    setItem: () => {
      writeCount += 1;
      throw new Error("quota exceeded");
    },
  };
  const dedupe = new ThreadReplyNotificationDedupe("user", storage);

  dedupe.reconcileActiveChannels(["channel"]);
  dedupe.setChannelReplayFloor("channel", 0);
  for (let index = 0; index < 100; index += 1) {
    assert.equal(dedupe.record("channel", `event-${index}`, index), true);
  }

  assert.equal(writeCount, 1);
  assert.equal(dedupe.record("channel", "event-99", 99), false);
});
