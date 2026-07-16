/**
 * Integration-seam tests for getConfigNudgeAuthorPubkey.
 *
 * These tests exercise the exact field-selection seam used by MessageRow.
 * Ordinary user events cannot delegate authorship through tags, while a
 * relay-signed event may still have a display author distinct from its signer.
 * Config-nudge authentication must always use the signer.
 *
 * By constructing a real TimelineMessage via formatTimelineMessages and
 * passing it to getConfigNudgeAuthorPubkey — the same helper MessageRow
 * calls — we lock the actual seam.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";

import { formatTimelineMessages } from "../lib/formatTimelineMessages.ts";
import { getConfigNudgeAuthorPubkey } from "./configNudgeAuthPubkey.ts";

const CHANNEL_ID = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";

// The raw event signer is a human (not an agent).
const HUMAN_SIGNER =
  "1111111111111111111111111111111111111111111111111111111111111111";
// The attributed agent pubkey (appears in actor/p tag).
const AGENT_PUBKEY =
  "2222222222222222222222222222222222222222222222222222222222222222";
const RELAY_SECRET = new Uint8Array(32).fill(4);
const RELAY_SIGNER = getPublicKey(RELAY_SECRET);

// MessageRow passes a predicate combining the community known-agent set with
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

function makeRelayEvent(tags) {
  return finalizeEvent(
    {
      kind: 9,
      created_at: 1_700_000_000,
      content: "**Fizz** needs configuration.\n\n```buzz:config-nudge\n{}\n```",
      tags,
    },
    RELAY_SECRET,
  );
}

function format(event, relaySelfPubkey) {
  return formatTimelineMessages(
    [event],
    null,
    undefined,
    null,
    undefined,
    undefined,
    undefined,
    undefined,
    relaySelfPubkey,
  );
}

// ── Spoof-regression test ─────────────────────────────────────────────────────
//
// A human-signed kind:9 event carries an `actor` tag naming an agent. The
// timeline must ignore the untrusted attribution, and the config-nudge gate
// must continue authenticating the human signer.

test("signerIsHuman_actorTagAttributedToAgent_returnsUndefined", () => {
  const event = makeEvent({
    pubkey: HUMAN_SIGNER,
    tags: [
      ["h", CHANNEL_ID],
      ["actor", AGENT_PUBKEY],
    ],
  });

  const [msg] = format(event);

  assert.equal(
    msg.pubkey?.toLowerCase(),
    HUMAN_SIGNER,
    "an untrusted actor tag must not replace the visible author",
  );
  assert.equal(
    msg.signerPubkey,
    HUMAN_SIGNER,
    "signerPubkey must remain the raw event signer",
  );

  // The guard must reject: signer is human, not in AGENT_PUBKEYS.
  assert.equal(
    getConfigNudgeAuthorPubkey(msg, isKnownAgentPubkey),
    undefined,
    "human signer with actor-tag attribution to agent must NOT enable the card",
  );
});

test("relayDelegatesToAgent_relaySigner_returnsUndefined", () => {
  const event = makeRelayEvent([
    ["h", CHANNEL_ID],
    ["actor", AGENT_PUBKEY],
  ]);

  const [msg] = format(event, RELAY_SIGNER);

  assert.equal(msg.pubkey, AGENT_PUBKEY);
  assert.equal(msg.signerPubkey, RELAY_SIGNER);
  assert.equal(
    getConfigNudgeAuthorPubkey(msg, isKnownAgentPubkey),
    undefined,
    "relay delegation must not be treated as an agent signature",
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
