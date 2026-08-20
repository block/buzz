import assert from "node:assert/strict";
import test from "node:test";

import { parseManualWorkstreamWaits } from "../lib/workstreamWaits.ts";
import { resolveWorkstreamWaitLink, waitAge } from "./WorkstreamWaits.tsx";

const now = Date.parse("2026-08-19T16:00:00Z");

test("waitAge labels age unknown when no trustworthy timestamp exists", () => {
  assert.equal(waitAge(undefined, now), "Unknown");
  assert.equal(waitAge("not-a-timestamp", now), "Unknown");
});

test("waitAge reports elapsed age for a trustworthy timestamp", () => {
  assert.equal(waitAge("2026-08-19T15:59:30Z", now), "0m");
  assert.equal(waitAge("2026-08-19T14:00:00Z", now), "2h");
});

const REFERENCE = {
  kind: "github",
  rawUrl: "https://github.com/block/buzz/pull/6357",
  href: "https://github.com/block/buzz/pull/6357",
  identity: "github:block/buzz#6357",
  owner: "block",
  repository: "buzz",
  number: 6357,
};

test("malformed present message values stay non-clickable while absent values retain reference fallback", () => {
  for (const message of [null, {}, 7]) {
    const [malformed] = parseManualWorkstreamWaits([
      {
        key: "github:block/buzz#6357",
        actor: { kind: "person", name: "Logan" },
        reason: "Need a decision",
        message,
      },
    ]);
    assert.equal(malformed.messagePresent, true);
    assert.equal(resolveWorkstreamWaitLink(malformed, REFERENCE), null);
  }

  const [absent] = parseManualWorkstreamWaits([
    {
      key: "github:block/buzz#6357",
      actor: { kind: "person", name: "Logan" },
      reason: "Need a decision",
    },
  ]);
  assert.deepEqual(resolveWorkstreamWaitLink(absent, REFERENCE), {
    kind: "reference",
    reference: REFERENCE,
  });
});

test("valid present message destination wins over a same-key reference", () => {
  const message = `buzz://message?channel=123e4567-e89b-12d3-a456-426614174000&id=${"a".repeat(64)}`;
  const [wait] = parseManualWorkstreamWaits([
    {
      key: "github:block/buzz#6357",
      actor: { kind: "person", name: "Logan" },
      reason: "Need a decision",
      message,
    },
  ]);
  assert.deepEqual(resolveWorkstreamWaitLink(wait, REFERENCE), {
    kind: "message",
    destination: {
      channelId: "123e4567-e89b-12d3-a456-426614174000",
      messageId: "a".repeat(64),
      threadRootId: null,
    },
  });
});
