import assert from "node:assert/strict";
import test from "node:test";

import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";

import {
  buildDecisionCardContent,
  buildDecisionCardTags,
  buildDecisionResponseContent,
  buildDecisionResponseTags,
  parseDecisionCard,
  parseDecisionResponse,
} from "./decisionCards.ts";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const CARD_EVENT_ID = "a".repeat(64);
const ROOT_EVENT_ID = "b".repeat(64);

const card = {
  schema_version: 1,
  card_id: "550e8400-e29b-41d4-a716-446655440000",
  title: "Approve corrected redraft",
  situation: "Case #625 has a corrected draft.",
  recommendation: "Approve it.",
  proposed_action: "Record shadow intent only.",
  risk: "No external send.",
  record_url: "https://stomaton.example/cases/625",
  choices: ["approve", "redraft", "escalate", "reject"],
  expires_at: 2_000_000_000,
  shadow: true,
};
const ENCODED_CARD = JSON.stringify(card);
const PAYLOAD_HASH = bytesToHex(sha256(new TextEncoder().encode(ENCODED_CARD)));

function cardTags(payload) {
  const encoded = JSON.stringify(payload);
  const hash = bytesToHex(sha256(new TextEncoder().encode(encoded)));
  return [
    ["decision_card", encoded],
    ["payload_hash", hash],
  ];
}

test("parses a versioned decision card tag without consuming its Markdown fallback", () => {
  const parsed = parseDecisionCard([
    ["h", CHANNEL_ID],
    ["decision_card", ENCODED_CARD],
    ["payload_hash", PAYLOAD_HASH],
    ["shadow", "1"],
  ]);

  assert.deepEqual(parsed, { payload: card, payloadHash: PAYLOAD_HASH });
});

test("rejects a card whose payload hash does not match what is rendered", () => {
  assert.equal(
    parseDecisionCard([
      ["decision_card", ENCODED_CARD],
      ["payload_hash", "c".repeat(64)],
    ]),
    null,
  );
});

test("fails closed for malformed cards and unsupported choices", () => {
  assert.equal(parseDecisionCard([["decision_card", "not-json"]]), null);
  assert.equal(
    parseDecisionCard(cardTags({ ...card, choices: ["launch"] })),
    null,
  );
  assert.equal(
    parseDecisionCard(cardTags({ ...card, record_url: "javascript:alert(1)" })),
    null,
  );
});

test("builds a thread-preserving signed response envelope", () => {
  const tags = buildDecisionResponseTags({
    actionId: "4f34cd24-9d97-4e94-998d-c7d933542dbc",
    cardEventId: CARD_EVENT_ID,
    cardId: card.card_id,
    channelId: CHANNEL_ID,
    decision: "approve",
    note: "Shadow only.",
    payloadHash: PAYLOAD_HASH,
    rootEventId: ROOT_EVENT_ID,
  });

  assert.deepEqual(tags.slice(0, 3), [
    ["h", CHANNEL_ID],
    ["e", ROOT_EVENT_ID, "", "root"],
    ["e", CARD_EVENT_ID, "", "reply"],
  ]);
  const parsed = parseDecisionResponse(tags);
  assert.equal(parsed?.action_id, "4f34cd24-9d97-4e94-998d-c7d933542dbc");
  assert.equal(parsed?.decision, "approve");
  assert.equal(parsed?.payload_hash, PAYLOAD_HASH);
  assert.equal(parsed?.shadow, true);
});

test("response fallback is explicit about shadow delivery state", () => {
  assert.equal(
    buildDecisionResponseContent("approve", "Shadow only."),
    "✅ Approved — SHADOW / NOT DELIVERED\n\n> Shadow only.",
  );
});

test("builds a native shadow card envelope with a matching payload hash", () => {
  const tags = buildDecisionCardTags({
    channelId: CHANNEL_ID,
    payload: card,
  });

  assert.deepEqual(tags[0], ["h", CHANNEL_ID]);
  assert.equal(tags.find(([name]) => name === "shadow")?.[1], "1");
  const encoded = tags.find(([name]) => name === "decision_card")?.[1];
  const payloadHash = tags.find(([name]) => name === "payload_hash")?.[1];
  assert.equal(encoded, ENCODED_CARD);
  assert.equal(payloadHash, PAYLOAD_HASH);
  assert.deepEqual(parseDecisionCard(tags), {
    payload: card,
    payloadHash: PAYLOAD_HASH,
  });
  assert.match(buildDecisionCardContent(card), /SHADOW \/ NOT DELIVERED/);
});

test("adds DM recipients so the relay can route a native card", () => {
  const tags = buildDecisionCardTags({
    channelId: CHANNEL_ID,
    payload: card,
    recipientPubkeys: ["c".repeat(64), "c".repeat(64)],
  });

  assert.deepEqual(tags.slice(0, 2), [
    ["h", CHANNEL_ID],
    ["p", "c".repeat(64)],
  ]);
  assert.equal(tags.filter(([name]) => name === "p").length, 1);
});
