/**
 * Who the members sidebar offers a "Remove from channel" action for.
 *
 * The regression this pins: removal used to require key custody
 * (`isLocallyManagedBot` — the agent is in this desktop's managed-agent
 * registry). An agent the relay declares the viewer owns, but which has fallen
 * out of that registry, showed as theirs in the profile panel while the member
 * list offered no way to remove it — so stale bot memberships in a DM could not
 * be cleared from the UI at all.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { canRemoveChannelMember } from "./memberUtils.ts";

const ME = "a".repeat(64);
const BOT = "b".repeat(64);

function eligibility(overrides = {}) {
  return canRemoveChannelMember({
    memberPubkey: BOT,
    memberRole: "bot",
    selfRole: "member",
    currentPubkey: ME,
    isLocallyManagedBot: false,
    viewerIsDeclaredOwner: false,
    ...overrides,
  });
}

describe("channel member removal eligibility", () => {
  it("lets a declared owner remove an agent missing from the local registry", () => {
    // The stale-bot case from the report: no key custody, relay says it's mine.
    assert.equal(
      eligibility({ isLocallyManagedBot: false, viewerIsDeclaredOwner: true }),
      true,
    );
  });

  it("still lets an owner remove a locally managed bot", () => {
    assert.equal(
      eligibility({ isLocallyManagedBot: true, viewerIsDeclaredOwner: false }),
      true,
    );
  });

  it("does not let an ordinary member remove someone else's agent", () => {
    assert.equal(eligibility(), false);
  });

  it("requires channel membership before owner-declared removal applies", () => {
    // A viewer who is not in the channel has no removal path, however the
    // relay labels the agent.
    assert.equal(
      eligibility({ selfRole: undefined, viewerIsDeclaredOwner: true }),
      false,
    );
  });

  // ── Pre-existing rules, unchanged ────────────────────────────────────────

  it("lets anyone remove themselves, even outside the channel roster", () => {
    assert.equal(
      eligibility({ memberPubkey: ME, selfRole: undefined }),
      true,
      "leaving is always yours to do",
    );
  });

  it("lets an admin remove any other member", () => {
    assert.equal(
      eligibility({ selfRole: "admin", memberRole: "member" }),
      true,
    );
    assert.equal(eligibility({ selfRole: "admin", memberRole: "owner" }), true);
  });

  it("lets an owner remove everyone except another owner", () => {
    assert.equal(
      eligibility({ selfRole: "owner", memberRole: "member" }),
      true,
    );
    assert.equal(
      eligibility({ selfRole: "owner", memberRole: "owner" }),
      false,
    );
  });

  it("an owner can still remove another owner's seat if it is their own agent", () => {
    assert.equal(
      eligibility({
        selfRole: "owner",
        memberRole: "owner",
        viewerIsDeclaredOwner: true,
      }),
      true,
    );
  });

  it("treats a missing current pubkey as no self-removal shortcut", () => {
    assert.equal(
      eligibility({ memberPubkey: BOT, currentPubkey: undefined }),
      false,
    );
  });
});
