import assert from "node:assert/strict";
import test from "node:test";

import {
  createArtilleryRefereeLeaseEvent,
  formatArtilleryRefereeLeaseMessage,
  parseArtilleryRefereeLeaseEvent,
  recoverArtilleryRefereeLease,
} from "./refereeLease.ts";

function lease(action, ownerId, term, now, leaseMs = 1_000) {
  return createArtilleryRefereeLeaseEvent({
    action,
    leaseMs,
    matchId: "match-1",
    now,
    ownerId,
    term,
  });
}

test("round-trips a channel-backed referee lease", () => {
  const event = lease("claim", "host-a", 1, 1_000);
  assert.deepEqual(
    parseArtilleryRefereeLeaseEvent(formatArtilleryRefereeLeaseMessage(event)),
    event,
  );
  assert.equal(parseArtilleryRefereeLeaseEvent("ordinary message"), null);
});

test("expires, renews, and releases one lease term", () => {
  const claim = lease("claim", "host-a", 1, 1_000);
  assert.equal(
    recoverArtilleryRefereeLease([claim], "match-1", 1_500)?.active,
    true,
  );
  assert.equal(
    recoverArtilleryRefereeLease([claim], "match-1", 2_001)?.active,
    false,
  );

  const renew = lease("renew", "host-a", 1, 1_800);
  assert.equal(
    recoverArtilleryRefereeLease([claim, renew], "match-1", 2_500)?.active,
    true,
  );
  const release = lease("release", "host-a", 1, 2_600);
  assert.equal(
    recoverArtilleryRefereeLease([claim, renew, release], "match-1", 2_600)
      ?.active,
    false,
  );
});

test("fences an old host and deterministically elects simultaneous claimants", () => {
  const events = [
    lease("claim", "old-host", 1, 1_000),
    lease("claim", "host-z", 2, 3_000),
    lease("claim", "host-a", 2, 3_000),
    lease("renew", "host-z", 2, 3_200),
    lease("renew", "old-host", 1, 3_400),
  ];
  const recovered = recoverArtilleryRefereeLease(events, "match-1", 3_500);

  assert.equal(recovered?.term, 2);
  assert.equal(recovered?.ownerId, "host-a");
  assert.equal(recovered?.active, true);
});
