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

test("private channels accept adds from every active role that can post", () => {
  for (const selfRole of ["owner", "admin", "member", "bot"]) {
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

  assert.equal(
    canAddChannelMembers({
      channelType: "stream",
      visibility: "private",
      selfRole: null,
    }),
    false,
    "a non-member must not be able to add",
  );
});

test("guests never add anyone, as the relay's decide_member_put refuses", () => {
  for (const visibility of ["open", "private"]) {
    assert.equal(
      canAddChannelMembers({
        channelType: "stream",
        visibility,
        selfRole: "guest",
      }),
      false,
      `a guest must not be offered adds (${visibility} channel)`,
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

test("unknown visibility fails closed", () => {
  assert.equal(
    canAddChannelMembers({ channelType: "stream", selfRole: "member" }),
    false,
  );
  assert.equal(
    canAddChannelMembers({ channelType: "stream", selfRole: "owner" }),
    false,
  );
});
