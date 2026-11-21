/**
 * Mounted archive readers keep their channel resident: the archive cap only
 * evicts channels no mounted `useArchivedChannelEvents` caller is viewing.
 * The profile activity carousel mounts one reader per channel slide, so more
 * channels can be on screen than the unviewed-channel cap.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

const tauriMock = {
  invoke(command) {
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const {
  ingestArchivedObserverEvents,
  resetAgentObserverStore,
  _testGetArchivedChannelEvents,
  _testRegisterKnownAgents,
} = await import("../observerRelayStore.ts");
const { useArchivedChannelEvents } = await import("./useObserverEvents.ts");

const AGENT = "a".repeat(64);
const PAGE = 50;
const VIEWED = ["view-0", "view-1", "view-2", "view-3", "view-4", "view-5"];

const decryptContent = (raw) => Promise.resolve(JSON.parse(raw.content));
function channelPage(channelId) {
  return Array.from({ length: PAGE }, (_, i) => ({
    id: `${channelId}-${i}`,
    pubkey: AGENT,
    created_at: 1000 + i,
    kind: 24200,
    tags: [
      ["agent", AGENT],
      ["frame", "telemetry"],
    ],
    content: JSON.stringify({
      seq: i + 1,
      timestamp: new Date(Date.UTC(2026, 0, 1, 0, 0, i)).toISOString(),
      kind: "acp_write",
      agentIndex: 0,
      channelId,
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: {},
    }),
    sig: "s".repeat(128),
  }));
}

const rendered = new Map();
function Reader({ channelId }) {
  rendered.set(channelId, useArchivedChannelEvents(AGENT, channelId).length);
  return null;
}

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  rendered.clear();
});

const retained = (channelId) =>
  _testGetArchivedChannelEvents(AGENT, channelId).length;

test("mounted readers keep their archives past the cap, then release them", async () => {
  resetAgentObserverStore();
  _testRegisterKnownAgents("sub", [AGENT]);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() =>
    root.render(
      VIEWED.map((channelId) =>
        React.createElement(Reader, { channelId, key: channelId }),
      ),
    ),
  );

  for (const channelId of VIEWED) {
    await act(() =>
      ingestArchivedObserverEvents(channelPage(channelId), decryptContent),
    );
  }
  // A thread panel elsewhere then browses many other channels.
  for (let c = 0; c < 10; c++) {
    await act(() =>
      ingestArchivedObserverEvents(channelPage(`other-${c}`), decryptContent),
    );
  }

  for (const channelId of VIEWED) {
    assert.equal(
      retained(channelId),
      PAGE,
      `${channelId} evicted while viewed`,
    );
    assert.equal(rendered.get(channelId), PAGE, `${channelId} rendered empty`);
  }
  assert.equal(retained("other-0"), 0, "unviewed channels stay bounded");

  act(() => root.unmount());
  root = null;
  const kept = [
    ...VIEWED,
    ...Array.from({ length: 10 }, (_, c) => `other-${c}`),
  ].filter((channelId) => retained(channelId) > 0);
  assert.equal(kept.length, 4, `after release only the cap remains: ${kept}`);
  // Released channels are newer than the channels browsed while they were
  // viewed, so the released ones are what remains.
  assert.ok(
    kept.every((channelId) => VIEWED.includes(channelId)),
    `unviewed channels browsed earlier must go first: ${kept}`,
  );
});
