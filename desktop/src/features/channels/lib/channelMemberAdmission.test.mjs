import { strict as assert } from "node:assert";
import test from "node:test";

import { canAddChannelMembers } from "./channelMemberAdmission.ts";

test("open channels accept adds from anyone, member or not", () => {
  assert.equal(
    canAddChannelMembers({
      channelType: "stream",
      visibility: "open",
      selfRole: null,
    }),
    true,
  );
  assert.equal(
    canAddChannelMembers({
      channelType: "stream",
      visibility: "open",
      selfRole: "member",
    }),
    true,
  );
});

test("private channels accept adds only from owners/admins", () => {
  for (const selfRole of ["owner", "admin"]) {
    assert.equal(
      canAddChannelMembers({
        channelType: "stream",
        visibility: "private",
        selfRole,
      }),
      true,
      `${selfRole} should be able to add`,
    );
  }

  for (const selfRole of ["member", "bot", "guest", null]) {
    assert.equal(
      canAddChannelMembers({
        channelType: "stream",
        visibility: "private",
        selfRole,
      }),
      false,
      `${selfRole} must not be able to add`,
    );
  }
});

test("DMs never accept adds, even from an owner", () => {
  assert.equal(
    canAddChannelMembers({
      channelType: "dm",
      visibility: "private",
      selfRole: "owner",
    }),
    false,
  );
  assert.equal(
    canAddChannelMembers({
      channelType: "dm",
      visibility: "open",
      selfRole: "owner",
    }),
    false,
  );
});

test("unknown visibility fails closed for non-elevated callers", () => {
  assert.equal(
    canAddChannelMembers({ channelType: "stream", selfRole: "member" }),
    false,
  );
  assert.equal(
    canAddChannelMembers({ channelType: "stream", selfRole: "owner" }),
    true,
  );
});
