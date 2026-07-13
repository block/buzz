/**
 * Integration-seam tests for getConfigNudgeAuthorPubkey.
 *
 * These tests exercise the exact field-selection seam that was broken:
 * formatTimelineMessages retains signerPubkey = raw event.pubkey while
 * resolving message.pubkey to the tag-attributed author. A test that
 * passes the author pubkey by hand (as the previous regression did) would
 * not catch a regression that reverts MessageRow to using message.pubkey
 * (the spoofable field) instead of signerPubkey.
 *
 * By constructing a real TimelineMessage via formatTimelineMessages and
 * passing it to getConfigNudgeAuthorPubkey — the same helper MessageRow
 * calls — we lock the actual seam.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { formatTimelineMessages } from "../lib/formatTimelineMessages.ts";
import { getConfigNudgeAuthorPubkey } from "./configNudgeAuthPubkey.ts";

const CHANNEL_ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

// The raw event signer is a human (not an agent).
const HUMAN_SIGNER =
  "1111111111111111111111111111111111111111111111111111111111111111";
// The attributed agent pubkey (appears in actor/p tag).
const AGENT_PUBKEY =
  "2222222222222222222222222222222222222222222222222222222222222222";

// MessageRow passes a predicate combining the workspace known-agent set with
// per-pubkey profile `isAgent` checks; the set-membership form is the minimal
// equivalent for exercising the signer-selection seam.
const AGENT_PUBKEYS = new Set([AGENT_PUBKEY]);
const isKnownAgentPubkey = (pubkey) => AGENT_PUBKEYS.has(pubkey);

function makeEvent(overrides = {}) {
  return {
    id: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    pubkey: HUMAN_SIGNER,
    kind: 9,
    created_at: 1_700_000_000,
    content: "**Fizz** needs configuration.\n\n```buzz:config-nudge\n{}\n```",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
    ...overrides,
  };
}

function format(event) {
  return formatTimelineMessages([event], null, undefined, null);
}

// ── Spoof-regression test ─────────────────────────────────────────────────────
//
// A human-signed kind:9 event carries an `actor` tag attributing it to the
// agent. formatTimelineMessages resolves message.pubkey = agent (via the
// actor tag) but retains signerPubkey = human (from raw event.pubkey).
// getConfigNudgeAuthorPubkey MUST return undefined — the human signer is
// not in the known-agent set, so the card must not render.

test("signerIsHuman_actorTagAttributedToAgent_returnsUndefined", () => {
  const event = makeEvent({
    pubkey: HUMAN_SIGNER,
    tags: [
      ["h", CHANNEL_ID],
      ["actor", AGENT_PUBKEY],
    ],
  });

  const [msg] = format(event);

  // Verify the seam: pubkey resolves to agent (tag-attributed), signerPubkey
  // is human (raw signer). If these are equal the test loses its meaning.
  assert.equal(
    msg.pubkey?.toLowerCase(),
    AGENT_PUBKEY,
    "attributed pubkey must be the agent (actor tag wins in resolveEventAuthorPubkey)",
  );
  assert.notEqual(
    msg.signerPubkey,
    AGENT_PUBKEY,
    "signerPubkey must NOT be the agent (event was signed by human)",
  );

  // The guard must reject: signer is human, not in AGENT_PUBKEYS.
  assert.equal(
    getConfigNudgeAuthorPubkey(msg, isKnownAgentPubkey),
    undefined,
    "human signer with actor-tag attribution to agent must NOT enable the card",
  );
});

// ── Positive case ─────────────────────────────────────────────────────────────
//
// A genuine kind:9 signed by the agent itself: getConfigNudgeAuthorPubkey
// must return the agent pubkey so MessageRow enables the card.

test("signerIsAgent_genuine_returnsAgentPubkey", () => {
  const event = makeEvent({ pubkey: AGENT_PUBKEY });

  const [msg] = format(event);

  assert.equal(
    msg.signerPubkey,
    AGENT_PUBKEY,
    "signerPubkey must be the agent when the event is signed by the agent",
  );

  assert.equal(
    getConfigNudgeAuthorPubkey(msg, isKnownAgentPubkey),
    AGENT_PUBKEY,
    "genuine agent-signed kind:9 must enable the card",
  );
});

// ── Non-kind:9 is always excluded ─────────────────────────────────────────────
//
// KIND_STREAM_MESSAGE_V2 (40002) is a valid timeline-content kind but is NOT
// kind:9 — the helper must return undefined even when the signer is a known
// agent, because the config-nudge sentinel is only emitted by the setup-listener
// on kind:9 (KIND_STREAM_MESSAGE) events.

test("nonKind9_agentSigner_returnsUndefined", () => {
  // kind 40002 = KIND_STREAM_MESSAGE_V2: a valid timeline event, not kind:9.
  const event = makeEvent({ pubkey: AGENT_PUBKEY, kind: 40002 });

  const [msg] = format(event);

  assert.equal(
    getConfigNudgeAuthorPubkey(msg, isKnownAgentPubkey),
    undefined,
    "non-kind:9 events must never enable the card even if signer is known agent",
  );
});
