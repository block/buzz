import assert from "node:assert/strict";
import test from "node:test";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
} from "nostr-tools/pure";

import { KIND_COFOUNDER_LEAD_HEAD } from "@/shared/constants/kinds";
import {
  COFOUNDER_LEAD_D_TAG,
  parseCofounderLeadStatus,
} from "./leadStatusRecords.ts";

const relaySecret = generateSecretKey();
const relayPubkey = getPublicKey(relaySecret);

function statusEvent(content, overrides = {}) {
  return finalizeEvent(
    {
      kind: KIND_COFOUNDER_LEAD_HEAD,
      content: JSON.stringify(content),
      tags: [["d", COFOUNDER_LEAD_D_TAG]],
      created_at: 1_791_273_780,
      ...overrides,
    },
    relaySecret,
  );
}

const unreachableContent = {
  schemaVersion: 1,
  state: "unreachable",
  problem: "several_scouts",
  detail: "The relay found more than one Scout.",
  attempts: 3,
  since: 1_791_273_600,
  updatedAt: 1_791_273_780,
};

test("parses the relay's unreachable Scout status", () => {
  const event = statusEvent(unreachableContent);
  const parsed = parseCofounderLeadStatus(event, relayPubkey);
  assert.deepEqual(parsed, {
    state: unreachableContent.state,
    problem: unreachableContent.problem,
    detail: unreachableContent.detail,
    attempts: unreachableContent.attempts,
    since: unreachableContent.since,
    updatedAt: unreachableContent.updatedAt,
    eventId: event.id,
  });
});

test("rejects untrusted, mis-scoped and internally inconsistent status heads", () => {
  const valid = statusEvent(unreachableContent);
  assert.equal(parseCofounderLeadStatus(valid, "f".repeat(64)), null);
  assert.equal(
    parseCofounderLeadStatus(
      statusEvent(unreachableContent, {
        tags: [["d", "company:cofounder"]],
      }),
      relayPubkey,
    ),
    null,
  );
  assert.equal(
    parseCofounderLeadStatus(
      statusEvent({ ...unreachableContent, problem: null }),
      relayPubkey,
    ),
    null,
  );
  assert.equal(
    parseCofounderLeadStatus(
      statusEvent({ ...unreachableContent, attempts: 0x1_0000_0000 }),
      relayPubkey,
    ),
    null,
  );
});
