import assert from "node:assert/strict";
import test from "node:test";

import { canAddChannelMembers } from "../../features/channels/lib/channelMemberAdmission.ts";

test("ACL: open channels allow add without membership; private needs member", () => {
  assert.equal(
    canAddChannelMembers({
      channelType: "channel",
      visibility: "open",
      selfRole: null,
    }),
    true,
  );
  assert.equal(
    canAddChannelMembers({
      channelType: "channel",
      visibility: "private",
      selfRole: null,
    }),
    false,
  );
  assert.equal(
    canAddChannelMembers({
      channelType: "channel",
      visibility: "private",
      selfRole: "member",
    }),
    true,
  );
  assert.equal(
    canAddChannelMembers({
      channelType: "dm",
      visibility: "private",
      selfRole: "member",
    }),
    false,
  );
});
