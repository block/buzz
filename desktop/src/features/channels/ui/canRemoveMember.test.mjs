import assert from "node:assert/strict";
import test from "node:test";

import { canRemoveMember } from "./canRemoveMember.ts";

const SELF = "self-pubkey";
const OTHER_OWNER = "other-owner-pubkey";
const MEMBER = "member-pubkey";
const BOT = "bot-pubkey";
const noBot = () => false;

test("owner can remove ANOTHER owner (the fix)", () => {
  const self = { role: "owner" };
  assert.equal(
    canRemoveMember(self, { role: "owner", pubkey: OTHER_OWNER }, SELF, noBot),
    true,
  );
});

test("owner can remove admins and ordinary members", () => {
  const self = { role: "owner" };
  assert.equal(
    canRemoveMember(self, { role: "admin", pubkey: MEMBER }, SELF, noBot),
    true,
  );
  assert.equal(
    canRemoveMember(self, { role: "member", pubkey: MEMBER }, SELF, noBot),
    true,
  );
});

test("self-removal preserved for every role", () => {
  for (const role of ["owner", "admin", "member"]) {
    assert.equal(
      canRemoveMember({ role }, { role, pubkey: SELF }, SELF, noBot),
      true,
      `${role} should be able to remove self`,
    );
  }
});

test("admin behavior unchanged: removes any non-self, never self", () => {
  const self = { role: "admin" };
  assert.equal(
    canRemoveMember(self, { role: "member", pubkey: MEMBER }, SELF, noBot),
    true,
  );
  assert.equal(
    canRemoveMember(self, { role: "owner", pubkey: OTHER_OWNER }, SELF, noBot),
    true,
  );
});

test("non-elevated member cannot remove other members", () => {
  const self = { role: "member" };
  assert.equal(
    canRemoveMember(self, { role: "owner", pubkey: OTHER_OWNER }, SELF, noBot),
    false,
  );
  assert.equal(
    canRemoveMember(self, { role: "admin", pubkey: MEMBER }, SELF, noBot),
    false,
  );
  assert.equal(
    canRemoveMember(self, { role: "member", pubkey: MEMBER }, SELF, noBot),
    false,
  );
});

test("owned-bot removal unchanged (any member may remove a bot they own)", () => {
  const self = { role: "member" };
  const isMyBot = (m) => m.pubkey === BOT;
  assert.equal(
    canRemoveMember(self, { role: "bot", pubkey: BOT }, SELF, isMyBot),
    true,
  );
  assert.equal(
    canRemoveMember(self, { role: "bot", pubkey: "not-my-bot" }, SELF, isMyBot),
    false,
  );
});
