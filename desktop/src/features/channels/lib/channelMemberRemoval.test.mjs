import { strict as assert } from "node:assert";
import test from "node:test";

import { canRemoveChannelMember } from "./channelMemberRemoval.ts";

const ME = "a".repeat(64);
const OTHER = "b".repeat(64);

function canRemove(overrides = {}) {
  return canRemoveChannelMember({
    memberPubkey: OTHER,
    memberRole: "member",
    currentPubkey: ME,
    selfRole: "member",
    isOwnAgent: false,
    ...overrides,
  });
}

test("anyone may remove themselves", () => {
  for (const selfRole of ["owner", "admin", "member", "guest", null]) {
    assert.equal(canRemove({ memberPubkey: ME, selfRole }), true, selfRole);
  }
});

test("admins and owners remove members, members only their own agents", () => {
  for (const memberRole of ["admin", "member", "guest", "bot"]) {
    assert.equal(canRemove({ selfRole: "admin", memberRole }), true);
    assert.equal(canRemove({ selfRole: "owner", memberRole }), true);
  }
  assert.equal(canRemove({ selfRole: "member" }), false);
  assert.equal(
    canRemove({ selfRole: "member", memberRole: "bot", isOwnAgent: true }),
    true,
  );
  assert.equal(
    canRemove({ selfRole: null, memberRole: "bot", isOwnAgent: true }),
    false,
    "a non-member has no removal authority, even over their own agent",
  );
});

test("only an owner removes an owner, as the relay's classify_remove_other requires", () => {
  assert.equal(
    canRemove({ selfRole: "admin", memberRole: "owner" }),
    false,
    "an admin must not be offered removing an owner",
  );
  assert.equal(
    canRemove({ selfRole: "member", memberRole: "owner", isOwnAgent: true }),
    false,
    "a member must not be offered removing their own agent once it is an owner",
  );
  assert.equal(
    canRemove({ selfRole: "owner", memberRole: "owner", isOwnAgent: true }),
    true,
    "an owner may remove their own agent that is an owner",
  );
});
