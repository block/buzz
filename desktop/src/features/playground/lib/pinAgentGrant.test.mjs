import assert from "node:assert/strict";
import test from "node:test";

import { findGrantForPlaygroundPin } from "./pinAgentGrant.ts";

test("findGrantForPlaygroundPin matches surfaceId or playground- webviewLabel", () => {
  const grants = [
    {
      webviewLabel: "playground-demo-1",
      surface: "playground",
      surfaceId: "demo-1",
      agentId: "a1",
      agentPubkey: "pk",
      channelId: "chan",
      mode: "drive",
      createdAtMs: 1,
    },
    {
      webviewLabel: "playground-other",
      surface: "playground",
      surfaceId: "other",
      agentId: "a2",
      agentPubkey: "pk2",
      channelId: "chan",
      mode: "observe",
      createdAtMs: 2,
    },
  ];
  assert.equal(findGrantForPlaygroundPin(grants, "demo-1")?.mode, "drive");
  assert.equal(findGrantForPlaygroundPin(grants, "other")?.mode, "observe");
  assert.equal(findGrantForPlaygroundPin(grants, "missing"), null);
});
