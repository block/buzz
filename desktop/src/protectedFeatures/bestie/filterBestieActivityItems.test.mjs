import assert from "node:assert/strict";
import test from "node:test";

import { filterBestieActivityItems } from "./filterBestieActivityItems.ts";

function tool(id, channelId = "dm-1", timestamp = "2026-01-01T00:00:00.000Z") {
  return {
    id,
    type: "tool",
    renderClass: "file-read",
    descriptor: { renderClass: "file-read", label: "Read", preview: null },
    title: "Read",
    toolName: "Read",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp,
    startedAt: timestamp,
    completedAt: timestamp,
    channelId,
  };
}

test("filterBestieActivityItems keeps tools, drops messages and other channels", () => {
  const items = [
    tool("t1", "dm-1"),
    {
      id: "m1",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Assistant",
      text: "hi",
      timestamp: "2026-01-01T00:00:00.000Z",
      channelId: "dm-1",
    },
    tool("t2", "other"),
    {
      id: "raw",
      type: "metadata",
      renderClass: "raw-rail",
      title: "Raw",
      sections: [],
      timestamp: "2026-01-01T00:00:00.000Z",
      channelId: "dm-1",
    },
  ];
  const filtered = filterBestieActivityItems(items, { channelId: "dm-1" });
  assert.deepEqual(
    filtered.map((item) => item.id),
    ["t1"],
  );
});

test("filterBestieActivityItems returns empty when session boundary is null", () => {
  const filtered = filterBestieActivityItems([tool("t1")], {
    channelId: "dm-1",
    sessionBoundary: null,
  });
  assert.deepEqual(filtered, []);
});

test("filterBestieActivityItems scopes to session start (Close Thread boundary)", () => {
  // Session started at unix 1000s → 1970-01-01T00:16:40.000Z
  const items = [
    tool("old", "dm-1", "1970-01-01T00:16:30.000Z"), // before
    tool("in", "dm-1", "1970-01-01T00:16:40.000Z"), // at boundary
    tool("later", "dm-1", "1970-01-01T00:17:00.000Z"),
  ];
  const filtered = filterBestieActivityItems(items, {
    channelId: "dm-1",
    sessionBoundary: { firstMessageCreatedAt: 1000 },
  });
  assert.deepEqual(
    filtered.map((item) => item.id),
    ["in", "later"],
  );
});
