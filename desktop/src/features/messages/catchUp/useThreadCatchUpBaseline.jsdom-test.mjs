/**
 * The recap baseline is frozen during render, before this visit's mark-read
 * effects (channel open, thread open, revealed replies) can move it.
 *
 * Falsifiability: capturing in an effect, or re-reading markers on every
 * render, makes the snapshots below report the post-visit markers.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { resolveCatchUpBaseline } from "./threadCatchUp.ts";
import { useThreadCatchUpBaseline } from "./useThreadCatchUpBaseline.ts";
import { useThreadCatchUpReadHandoff } from "./useThreadCatchUpReadHandoff.ts";

const CHANNEL = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const ROOT = "f".repeat(64);
const REPLY_A = "a".repeat(64);
const REPLY_B = "b".repeat(64);
const ALICE = "c".repeat(64);
const NOW = 2_000_000;

function Harness({
  markers,
  ready,
  threadMessageIds,
  snapshots,
  markOnOpen,
  inboxSnapshot,
}) {
  const snapshot = useThreadCatchUpBaseline({
    channelId: CHANNEL,
    rootEventId: ROOT,
    threadMessageIds,
    getOwnReadAt: (contextId) => markers.get(contextId) ?? null,
    isReadStateReady: ready,
    inboxSnapshot,
  });
  snapshots.push(snapshot);
  // Mirrors useChannelOpenReadState and the thread-open reply marking.
  React.useEffect(() => {
    if (!markOnOpen) return;
    markers.set(CHANNEL, NOW);
    for (const id of threadMessageIds) markers.set(`msg:${id}`, NOW);
  });
  return null;
}

test("markers advanced by visit effects never become the previous read position", async () => {
  const markers = new Map([
    [CHANNEL, NOW - 7_200],
    [`msg:${REPLY_A}`, NOW - 7_200],
  ]);
  const snapshots = [];
  const root = createRoot(document.createElement("div"));
  const props = { markers, ready: true, snapshots, markOnOpen: true };
  await act(async () => {
    root.render(
      React.createElement(Harness, {
        ...props,
        threadMessageIds: [ROOT, REPLY_A],
      }),
    );
  });
  assert.equal(markers.get(CHANNEL), NOW, "effect ran");
  // A reply loads after the visit marked everything visible.
  await act(async () => {
    root.render(
      React.createElement(Harness, {
        ...props,
        threadMessageIds: [ROOT, REPLY_A, REPLY_B],
      }),
    );
  });
  const latest = snapshots.at(-1);
  assert.equal(latest.channelReadAt, NOW - 7_200);
  assert.equal(latest.replyReadAt.get(REPLY_A), NOW - 7_200);
  assert.equal(latest.replyReadAt.get(REPLY_B), null);
  assert.equal(latest.repliesReady, true);

  const baseline = resolveCatchUpBaseline({
    snapshot: latest,
    replies: [
      { id: REPLY_A, createdAt: NOW - 3_600, pubkey: ALICE },
      { id: REPLY_B, createdAt: NOW - 60, pubkey: ALICE },
    ],
    nowSeconds: NOW,
  });
  assert.deepEqual(baseline, {
    kind: "marker",
    since: NOW - 3_600,
    previousReadAt: NOW - 7_200,
    unreadCount: 2,
  });
  await act(async () => root.unmount());
});

test("captures taken before read state loads stay unresolved after it loads", async () => {
  const markers = new Map();
  const snapshots = [];
  const root = createRoot(document.createElement("div"));
  const base = {
    markers,
    snapshots,
    markOnOpen: false,
    threadMessageIds: [ROOT, REPLY_A],
  };
  await act(async () => {
    root.render(React.createElement(Harness, { ...base, ready: false }));
  });
  markers.set(CHANNEL, NOW - 60);
  await act(async () => {
    root.render(React.createElement(Harness, { ...base, ready: true }));
  });
  const latest = snapshots.at(-1);
  assert.equal(latest.channelReady, false);
  assert.equal(latest.channelReadAt, null);
  assert.deepEqual(
    resolveCatchUpBaseline({ snapshot: latest, replies: [], nowSeconds: NOW }),
    { kind: "unresolved" },
  );
  await act(async () => root.unmount());
});

// The browser journey exercises HomeView's real onSelect. This covers the
// readiness and identity transitions that must not reuse a navigation snapshot.
test("Inbox handoff preserves absent markers and cannot cross identity or community", async () => {
  const markers = new Map([[`thread:${ROOT}`, NOW - 7_200]]);
  let handoff;
  function Inbox({
    pubkey = ALICE,
    relayUrl = "ws://127.0.0.1:3303",
    ready = true,
  }) {
    handoff = useThreadCatchUpReadHandoff({
      relayUrl,
      pubkey,
      isReadStateReady: ready,
      getOwnReadAt: (key) => markers.get(key) ?? null,
    });
    return null;
  }
  const inbox = createRoot(document.createElement("div"));
  await act(async () => inbox.render(React.createElement(Inbox)));
  handoff.capture({
    channelId: CHANNEL,
    rootEventId: ROOT,
    messageIds: [REPLY_A],
  });
  const pending = handoff.peek(CHANNEL);
  assert.equal(pending.snapshot.channelReadAt, null);
  markers.set(CHANNEL, NOW);
  markers.set(`thread:${ROOT}`, NOW);
  markers.set(`msg:${REPLY_A}`, NOW);
  // Browsing another Inbox row and returning must retain A's first snapshot.
  handoff.capture({ channelId: CHANNEL, rootEventId: REPLY_B, messageIds: [] });
  handoff.capture({
    channelId: CHANNEL,
    rootEventId: ROOT,
    messageIds: [REPLY_A],
  });
  assert.equal(handoff.peek(CHANNEL).key, pending.key);
  handoff.capture({
    channelId: CHANNEL,
    rootEventId: ROOT,
    messageIds: [REPLY_A, REPLY_B],
  });
  assert.equal(
    handoff.peek(CHANNEL).snapshot.replyReadAt.get(REPLY_B),
    null,
    "a new reply is captured before the next Inbox mark-read",
  );

  const channel = createRoot(document.createElement("div"));
  const snapshots = [];
  await act(async () =>
    channel.render(
      React.createElement(Harness, {
        markers,
        ready: true,
        snapshots,
        markOnOpen: false,
        threadMessageIds: [ROOT, REPLY_A],
        inboxSnapshot: pending.snapshot,
      }),
    ),
  );
  const baseline = resolveCatchUpBaseline({
    snapshot: snapshots.at(-1),
    replies: [{ id: REPLY_A, createdAt: NOW - 3_600, pubkey: ALICE }],
    nowSeconds: NOW,
  });
  assert.deepEqual(baseline, {
    kind: "marker",
    since: NOW - 3_600,
    previousReadAt: NOW - 7_200,
    unreadCount: 1,
  });
  handoff.consume(pending.key);
  assert.equal(handoff.peek(CHANNEL), null);

  for (const scope of [
    { pubkey: "b".repeat(64) },
    { relayUrl: "ws://127.0.0.1:3304" },
  ]) {
    await act(async () => inbox.render(React.createElement(Inbox)));
    handoff.capture({ channelId: CHANNEL, rootEventId: ROOT, messageIds: [] });
    await act(async () => inbox.render(React.createElement(Inbox, scope)));
    assert.equal(handoff.peek(CHANNEL), null);
    await act(async () => inbox.render(React.createElement(Inbox)));
    assert.equal(
      handoff.peek(CHANNEL),
      null,
      "switching back cannot revive it",
    );
  }
  await act(async () =>
    inbox.render(React.createElement(Inbox, { ready: false })),
  );
  handoff.capture({ channelId: CHANNEL, rootEventId: ROOT, messageIds: [] });
  await act(async () =>
    inbox.render(React.createElement(Inbox, { ready: true })),
  );
  assert.equal(handoff.peek(CHANNEL).snapshot.channelReady, false);
  const realNow = Date.now;
  try {
    const now = realNow();
    Date.now = () => now + 5 * 60 * 1_000;
    assert.equal(handoff.peek(CHANNEL), null, "an abandoned snapshot expires");
  } finally {
    Date.now = realNow;
  }
  await act(async () => {
    inbox.unmount();
    channel.unmount();
  });
});
