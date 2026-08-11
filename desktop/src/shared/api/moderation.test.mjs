import assert from "node:assert/strict";
import test from "node:test";

import {
  moderationPubkeyToProtocolHex,
  toAction,
  toReport,
  toRestriction,
} from "./moderation.ts";

const HEX = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const NPUB = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

test("moderation REST identity fields normalize from npub to internal hex", () => {
  const report = toReport({
    id: "report-row",
    report_event_id: "11".repeat(32),
    reporter_pubkey: NPUB,
    target_kind: "pubkey",
    target: NPUB,
    channel_id: null,
    report_type: "spam",
    note: null,
    status: "open",
    resolved_by: NPUB,
    resolved_at: null,
    action_id: null,
    created_at: "2026-01-01T00:00:00Z",
  });
  assert.equal(report.reporterPubkey, HEX);
  assert.equal(report.target, HEX);
  assert.equal(report.resolvedBy, HEX);

  const action = toAction({
    id: "action-row",
    actor_pubkey: NPUB,
    action: "ban",
    target_pubkey: NPUB,
    target_event_id: null,
    channel_id: null,
    reason_code: null,
    public_reason: null,
    private_reason: null,
    matched_principal: null,
    created_at: "2026-01-01T00:00:00Z",
  });
  assert.equal(action.actorPubkey, HEX);
  assert.equal(action.targetPubkey, HEX);

  const restriction = toRestriction({
    pubkey: NPUB,
    banned: true,
    ban_expires_at: null,
    ban_reason: null,
    muted_until: null,
    mute_reason: null,
    actor_pubkey: NPUB,
    updated_at: "2026-01-01T00:00:00Z",
  });
  assert.equal(restriction.pubkey, HEX);
  assert.equal(restriction.actorPubkey, HEX);
});

test("moderation write identities normalize npub to NIP-01 tag hex", () => {
  assert.equal(moderationPubkeyToProtocolHex(NPUB), HEX);
  assert.equal(moderationPubkeyToProtocolHex(HEX.toUpperCase()), HEX);
  assert.throws(
    () => moderationPubkeyToProtocolHex("nsec1copy-paste-mistake"),
    /Expected a valid npub/,
  );
});
