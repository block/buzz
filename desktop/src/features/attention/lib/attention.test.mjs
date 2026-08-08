import assert from "node:assert/strict";
import test from "node:test";

import {
  actionPublishes,
  attentionReason,
  DONE_RETENTION_SECONDS,
  HANDLED_ELSEWHERE_REPLY,
  isAttentionWorthy,
  offersHandledElsewhere,
  persistableZoneState,
  primaryGenericAction,
  projectAttention,
  pruneZoneState,
} from "./attention.ts";

const NOW = 1_700_000_000;
const ASK_CONTENT = "Can you review the plan?";

function makeInboxItem(overrides = {}) {
  const {
    conversationId = "conv-1",
    categories = ["mention"],
    content = ASK_CONTENT,
    isActionRequired = false,
    latestActivityAt = NOW - 600,
    kind = 9,
    groupItems,
    tags = [],
  } = overrides;
  const item = {
    id: `${conversationId}-latest`,
    kind,
    pubkey: "a".repeat(64),
    content,
    createdAt: latestActivityAt,
    channelId: "channel-1",
    channelName: "general",
    tags,
    category: categories[0] ?? "mention",
  };
  return {
    avatarUrl: null,
    conversationId,
    id: item.id,
    item,
    categories,
    categoryLabel: "Mention",
    channelLabel: "general",
    fullTimestampLabel: "",
    groupItems: groupItems ?? [item],
    isActionRequired,
    latestActivityAt,
    mentionNames: [],
    preview: content,
    senderLabel: "Alice",
    subject: content,
    timestampLabel: "",
    unreadCount: 1,
  };
}

test("mention and needs_action items are attention-worthy, plain activity is not", () => {
  assert.equal(
    isAttentionWorthy(makeInboxItem({ categories: ["mention"] })),
    true,
  );
  assert.equal(
    isAttentionWorthy(
      makeInboxItem({ categories: ["needs_action"], isActionRequired: true }),
    ),
    true,
  );
  assert.equal(
    isAttentionWorthy(makeInboxItem({ categories: ["activity"] })),
    false,
  );
  assert.equal(
    isAttentionWorthy(makeInboxItem({ categories: ["agent_activity"] })),
    false,
  );
});

test("ask-bearing items land in Needs Me, ask-less mentions demote to Heads up", () => {
  const projection = projectAttention(
    [
      makeInboxItem({ conversationId: "conv-ask" }),
      makeInboxItem({ conversationId: "conv-hi", content: "hello" }),
    ],
    {},
    NOW,
  );
  assert.equal(projection.needsMe.length, 1);
  assert.equal(projection.needsMe[0].id, "conv-ask");
  assert.equal(projection.needsMe[0].zone, "needsMe");
  assert.equal(projection.needsMe[0].reactivated, false);
  assert.equal(projection.needsMe[0].askType, "review");
  assert.equal(projection.needsMe[0].ask, ASK_CONTENT);
  assert.equal(projection.needsMe[0].askCount, 1);

  assert.equal(projection.headsUp.length, 1);
  assert.equal(projection.headsUp[0].id, "conv-hi");
  assert.equal(projection.headsUp[0].askType, "headsUp");
  assert.equal(projection.headsUp[0].ask, null);

  assert.equal(projection.waiting.length, 0);
  assert.equal(projection.done.length, 0);
});

test("multi-ask messages carry askCount through the projection", () => {
  const projection = projectAttention(
    [
      makeInboxItem({
        content: "Can you approve the budget? Could you review the deck?",
      }),
    ],
    {},
    NOW,
  );
  assert.equal(projection.needsMe.length, 1);
  assert.equal(projection.needsMe[0].askCount, 2);
});

test("declared asks for the viewer beat the derived tier", () => {
  const content = [
    "Context paragraph that mentions review casually.",
    "**Needs Lee, decision:** Ship now or wait for the fix?",
    "- Ship it now.",
    "- Wait for the relay fix.",
  ].join("\n");
  const projection = projectAttention([makeInboxItem({ content })], {}, NOW, {
    viewerName: "Lee Ntshudisane",
  });
  assert.equal(projection.needsMe.length, 1);
  const item = projection.needsMe[0];
  assert.equal(item.askType, "decision");
  assert.equal(item.ask, "Ship now or wait for the fix?");
  assert.equal(item.askCount, 1);
  assert.equal(item.declaredAsks?.length, 1);
  assert.deepEqual(item.declaredAsks?.[0].options, [
    "Ship it now.",
    "Wait for the relay fix.",
  ]);
});

test("two declared asks for the viewer set askCount and declaredAsks", () => {
  const content = [
    "**Needs Lee, question:** Did the overnight backup run?",
    "**Needs Lee, review:** Review the retention change when you can.",
  ].join("\n");
  const projection = projectAttention([makeInboxItem({ content })], {}, NOW, {
    viewerName: "Lee",
  });
  assert.equal(projection.needsMe.length, 1);
  assert.equal(projection.needsMe[0].askCount, 2);
  assert.equal(projection.needsMe[0].declaredAsks?.length, 2);
  assert.equal(projection.needsMe[0].askType, "question");
});

test("messages with only non-viewer declarations demote to Heads up", () => {
  const content = [
    "**Needs Axel, decision:** Ship now or wait for the fix?",
    "- Ship it now.",
    "- Wait for the relay fix.",
  ].join("\n");
  const projection = projectAttention([makeInboxItem({ content })], {}, NOW, {
    viewerName: "Lee",
  });
  assert.equal(projection.needsMe.length, 0);
  assert.equal(projection.headsUp.length, 1);
  assert.equal(projection.headsUp[0].askType, "headsUp");
  assert.equal(projection.headsUp[0].declaredAsks, undefined);
});

test("without a viewer name the declared tier is inert", () => {
  const content = "**Needs Lee, decision:** Ship now or wait for the fix?";
  const projection = projectAttention([makeInboxItem({ content })], {}, NOW);
  // Falls to derived: the declaration is for someone else as far as the
  // projection knows, and stripping leaves no other ask.
  assert.equal(projection.needsMe.length, 0);
  assert.equal(projection.headsUp.length, 1);
});

test("Noted on a To note item is silent; every other action publishes", () => {
  // Regression pair: To note Noted publishes nothing…
  assert.equal(actionPublishes("headsUp", "noted"), false);
  // …and an actionable Noted (badge-corrected bucket) publishes.
  for (const type of [
    "decision",
    "approval",
    "question",
    "review",
    "blocked",
  ]) {
    assert.equal(actionPublishes(type, "noted"), true, `${type} noted`);
  }
  // Done and Waiting always publish, even from a heads-up-typed card.
  assert.equal(actionPublishes("headsUp", "done"), true);
  assert.equal(actionPublishes("headsUp", "waiting"), true);
  assert.equal(actionPublishes("decision", "done"), true);
  assert.equal(actionPublishes("question", "waiting"), true);
});

test("badge overrides rebucket items deterministically", () => {
  const overridden = projectAttention(
    [makeInboxItem({ content: "hello" })],
    {},
    NOW,
    { badgeOverrides: { "conv-1": "decision" } },
  );
  assert.equal(overridden.needsMe.length, 1);
  assert.equal(overridden.needsMe[0].askType, "decision");

  const demoted = projectAttention([makeInboxItem()], {}, NOW, {
    badgeOverrides: { "conv-1": "headsUp" },
  });
  assert.equal(demoted.needsMe.length, 0);
  assert.equal(demoted.headsUp.length, 1);
});

test("config-nudge noise demotes to Heads up", () => {
  const projection = projectAttention(
    [makeInboxItem({ content: "Please update buzz:config-nudge settings." })],
    {},
    NOW,
  );
  assert.equal(projection.needsMe.length, 0);
  assert.equal(projection.headsUp.length, 1);
  assert.equal(projection.headsUp[0].askType, "headsUp");
});

test("plain activity items are excluded from every view", () => {
  const projection = projectAttention(
    [makeInboxItem({ categories: ["activity"] })],
    {},
    NOW,
  );
  assert.equal(projection.needsMe.length, 0);
  assert.equal(projection.headsUp.length, 0);
  assert.equal(projection.waiting.length, 0);
  assert.equal(projection.done.length, 0);
});

test("kind 46010 is an approval, kind 40007 falls back to review", () => {
  const approval = projectAttention(
    [
      makeInboxItem({
        categories: ["needs_action"],
        isActionRequired: true,
        kind: 46010,
        content: "hello",
      }),
    ],
    {},
    NOW,
  );
  assert.equal(approval.needsMe.length, 1);
  assert.equal(approval.needsMe[0].askType, "approval");
  assert.notEqual(approval.needsMe[0].ask, null);

  const reminder = projectAttention(
    [
      makeInboxItem({
        categories: ["needs_action"],
        isActionRequired: true,
        kind: 40007,
        content: "hello",
      }),
    ],
    {},
    NOW,
  );
  assert.equal(reminder.needsMe.length, 1);
  assert.equal(reminder.needsMe[0].askType, "review");
});

test("a parked waiting item stays in Waiting while activity is older than the park", () => {
  const item = makeInboxItem({ latestActivityAt: NOW - 3_600 });
  const projection = projectAttention(
    [item],
    { "conv-1": { zone: "waiting", changedAt: NOW - 60 } },
    NOW,
  );
  assert.equal(projection.waiting.length, 1);
  assert.equal(projection.needsMe.length, 0);
  assert.equal(projection.waiting[0].zoneChangedAt, NOW - 60);
});

test("new activity after parking reactivates ask-bearing items into Needs Me", () => {
  const item = makeInboxItem({ latestActivityAt: NOW - 10 });
  for (const zone of ["waiting", "done"]) {
    const projection = projectAttention(
      [item],
      { "conv-1": { zone, changedAt: NOW - 3_600 } },
      NOW,
    );
    assert.equal(projection.needsMe.length, 1, `${zone} should reactivate`);
    assert.equal(projection.needsMe[0].reactivated, true);
    assert.equal(projection.headsUp.length, 0);
    assert.equal(projection.waiting.length, 0);
    assert.equal(projection.done.length, 0);
  }
});

test("new activity on an ask-less parked item reactivates into Heads up", () => {
  const item = makeInboxItem({ content: "hello", latestActivityAt: NOW - 10 });
  const projection = projectAttention(
    [item],
    { "conv-1": { zone: "waiting", changedAt: NOW - 3_600 } },
    NOW,
  );
  assert.equal(projection.needsMe.length, 0);
  assert.equal(projection.headsUp.length, 1);
  assert.equal(projection.headsUp[0].reactivated, true);
});

test("done items show in Done until retention expires, then disappear", () => {
  const item = makeInboxItem({ latestActivityAt: NOW - 100_000 });
  const fresh = projectAttention(
    [item],
    { "conv-1": { zone: "done", changedAt: NOW - 3_600 } },
    NOW,
  );
  assert.equal(fresh.done.length, 1);

  const staleItem = makeInboxItem({
    latestActivityAt: NOW - DONE_RETENTION_SECONDS - 100,
  });
  const expired = projectAttention(
    [staleItem],
    {
      "conv-1": {
        zone: "done",
        changedAt: NOW - DONE_RETENTION_SECONDS - 10,
      },
    },
    NOW,
  );
  assert.equal(expired.done.length, 0);
  assert.equal(expired.needsMe.length, 0);
  assert.equal(expired.headsUp.length, 0);
  assert.equal(expired.waiting.length, 0);
});

test("needs me sorts oldest first, waiting and done by park time", () => {
  const older = makeInboxItem({
    conversationId: "conv-old",
    latestActivityAt: NOW - 5_000,
  });
  const newer = makeInboxItem({
    conversationId: "conv-new",
    latestActivityAt: NOW - 100,
  });
  const needsMe = projectAttention([older, newer], {}, NOW).needsMe;
  assert.deepEqual(
    needsMe.map((entry) => entry.id),
    ["conv-old", "conv-new"],
  );

  const waiting = projectAttention(
    [older, newer],
    {
      "conv-old": { zone: "waiting", changedAt: NOW - 50 },
      "conv-new": { zone: "waiting", changedAt: NOW - 10 },
    },
    NOW,
  ).waiting;
  assert.deepEqual(
    waiting.map((entry) => entry.id),
    ["conv-new", "conv-old"],
  );
});

test("heads up sorts newest first", () => {
  const older = makeInboxItem({
    conversationId: "conv-old",
    content: "hello",
    latestActivityAt: NOW - 5_000,
  });
  const newer = makeInboxItem({
    conversationId: "conv-new",
    content: "hello",
    latestActivityAt: NOW - 100,
  });
  const headsUp = projectAttention([older, newer], {}, NOW).headsUp;
  assert.deepEqual(
    headsUp.map((entry) => entry.id),
    ["conv-new", "conv-old"],
  );
});

test("attentionReason maps categories and kinds to short phrases", () => {
  assert.equal(
    attentionReason(
      makeInboxItem({
        categories: ["needs_action"],
        isActionRequired: true,
        kind: 46010,
      }),
    ),
    "Approval requested",
  );
  assert.equal(
    attentionReason(
      makeInboxItem({
        categories: ["needs_action"],
        isActionRequired: true,
        kind: 40007,
      }),
    ),
    "Reminder due",
  );
  assert.equal(
    attentionReason(makeInboxItem({ categories: ["mention"] })),
    "Mentioned you",
  );
  const item = makeInboxItem({ categories: ["mention"] });
  const threaded = {
    ...item,
    groupItems: [item.item, { ...item.item, id: "second" }],
  };
  assert.equal(attentionReason(threaded), "Mentioned you in an active thread");
});

test("pruneZoneState drops expired done entries and caps the map", () => {
  const state = {
    "conv-live": { zone: "waiting", changedAt: NOW - 10 },
    "conv-done-fresh": { zone: "done", changedAt: NOW - 60 },
    "conv-done-stale": {
      zone: "done",
      changedAt: NOW - DONE_RETENTION_SECONDS - 60,
    },
  };
  const pruned = pruneZoneState(state, NOW);
  assert.deepEqual(Object.keys(pruned).sort(), [
    "conv-done-fresh",
    "conv-live",
  ]);

  const crowded = Object.fromEntries(
    Array.from({ length: 10 }, (_, index) => [
      `conv-${index}`,
      { zone: "waiting", changedAt: NOW - index },
    ]),
  );
  const capped = pruneZoneState(crowded, NOW, 3);
  assert.deepEqual(Object.keys(capped).sort(), ["conv-0", "conv-1", "conv-2"]);
});

test("persistableZoneState excludes held entries and persists the rest", () => {
  const state = {
    "conv-held": { zone: "done", changedAt: NOW - 2 },
    "conv-settled": { zone: "waiting", changedAt: NOW - 60 },
  };
  const filtered = persistableZoneState(state, new Set(["conv-held"]));
  assert.deepEqual(Object.keys(filtered), ["conv-settled"]);
  // No holds: same map back, nothing dropped.
  assert.equal(persistableZoneState(state, new Set()), state);
});

test("a reactivated card whose reply committed carries the responded flag", () => {
  const parkedAt = NOW - 600;
  const items = [
    makeInboxItem({
      conversationId: "conv-replied",
      latestActivityAt: NOW - 10,
    }),
    makeInboxItem({
      conversationId: "conv-silent",
      latestActivityAt: NOW - 10,
    }),
  ];
  const projection = projectAttention(
    items,
    {
      "conv-replied": {
        zone: "done",
        changedAt: parkedAt,
        respondedAt: parkedAt + 5,
      },
      "conv-silent": { zone: "done", changedAt: parkedAt },
    },
    NOW,
  );
  const replied = projection.needsMe.find((item) => item.id === "conv-replied");
  const silent = projection.needsMe.find((item) => item.id === "conv-silent");
  assert.equal(replied.reactivated, true);
  assert.equal(replied.responded, true);
  assert.equal(silent.reactivated, true);
  assert.equal(silent.responded, false);
});

test("an exact-entry restore keeps a reactivated card in Needs Me, a re-marked one hides it", () => {
  const parkedAt = NOW - 600;
  const item = makeInboxItem({ latestActivityAt: NOW - 300 });
  // Undo restores the original entry: activity is newer, card reactivates.
  const restored = projectAttention(
    [item],
    { "conv-1": { zone: "waiting", changedAt: parkedAt } },
    NOW,
  );
  assert.equal(restored.needsMe.length, 1);
  assert.equal(restored.needsMe[0].reactivated, true);
  // A revert that re-marked with a fresh timestamp would bury it in Waiting.
  const remarked = projectAttention(
    [item],
    { "conv-1": { zone: "waiting", changedAt: NOW } },
    NOW,
  );
  assert.equal(remarked.needsMe.length, 0);
  assert.equal(remarked.waiting.length, 1);
});

test("locked action matrix: generic primaries and the Handled-elsewhere offer", () => {
  assert.equal(primaryGenericAction("blocked"), "done");
  assert.equal(primaryGenericAction("headsUp"), "noted");
  for (const type of ["approval", "decision", "question", "review"]) {
    assert.equal(primaryGenericAction(type), null, `${type} primary`);
    assert.equal(offersHandledElsewhere(type), true, `${type} overflow`);
  }
  assert.equal(offersHandledElsewhere("blocked"), false);
  assert.equal(offersHandledElsewhere("headsUp"), false);
});

test("Handled elsewhere publishes the frozen line and counts as publishing", () => {
  assert.equal(
    HANDLED_ELSEWHERE_REPLY,
    "Handled outside this thread, no answer coming here. If you are still blocked, say so and I will pick it up.",
  );
  for (const type of ["approval", "decision", "question", "review"]) {
    assert.equal(actionPublishes(type, "handledElsewhere"), true, type);
  }
});
