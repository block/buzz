import assert from "node:assert/strict";
import { test } from "node:test";

import { moderationCapabilities } from "./capabilities.ts";

// ── owner / admin ─────────────────────────────────────────────────────────────

test("moderationCapabilities: owner gets all capabilities", () => {
  const caps = moderationCapabilities("owner");
  assert.strictEqual(caps.canDelete, true);
  assert.strictEqual(caps.canKick, true);
  assert.strictEqual(caps.canTimeout, true);
  assert.strictEqual(caps.canUntimeout, true);
  assert.strictEqual(caps.canBan, true);
  assert.strictEqual(caps.canUnban, true);
  assert.strictEqual(caps.canViewQueue, true);
  assert.strictEqual(caps.canResolve, true);
});

test("moderationCapabilities: admin gets all capabilities", () => {
  const caps = moderationCapabilities("admin");
  assert.strictEqual(caps.canDelete, true);
  assert.strictEqual(caps.canKick, true);
  assert.strictEqual(caps.canTimeout, true);
  assert.strictEqual(caps.canUntimeout, true);
  assert.strictEqual(caps.canBan, true);
  assert.strictEqual(caps.canUnban, true);
  assert.strictEqual(caps.canViewQueue, true);
  assert.strictEqual(caps.canResolve, true);
});

// ── moderator ─────────────────────────────────────────────────────────────────

test("moderationCapabilities: moderator gets delete/kick/timeout/untimeout/queue/resolve", () => {
  const caps = moderationCapabilities("moderator");
  assert.strictEqual(caps.canDelete, true);
  assert.strictEqual(caps.canKick, true);
  assert.strictEqual(caps.canTimeout, true);
  assert.strictEqual(caps.canUntimeout, true);
  assert.strictEqual(caps.canViewQueue, true);
  assert.strictEqual(caps.canResolve, true);
});

test("moderationCapabilities: moderator does NOT get ban or unban", () => {
  const caps = moderationCapabilities("moderator");
  assert.strictEqual(caps.canBan, false);
  assert.strictEqual(caps.canUnban, false);
});

// ── member / null / undefined ─────────────────────────────────────────────────

test("moderationCapabilities: member gets no capabilities", () => {
  const caps = moderationCapabilities("member");
  assert.strictEqual(caps.canDelete, false);
  assert.strictEqual(caps.canKick, false);
  assert.strictEqual(caps.canTimeout, false);
  assert.strictEqual(caps.canUntimeout, false);
  assert.strictEqual(caps.canBan, false);
  assert.strictEqual(caps.canUnban, false);
  assert.strictEqual(caps.canViewQueue, false);
  assert.strictEqual(caps.canResolve, false);
});

test("moderationCapabilities: null gets no capabilities", () => {
  const caps = moderationCapabilities(null);
  assert.strictEqual(caps.canBan, false);
  assert.strictEqual(caps.canDelete, false);
  assert.strictEqual(caps.canViewQueue, false);
});

test("moderationCapabilities: undefined gets no capabilities", () => {
  const caps = moderationCapabilities(undefined);
  assert.strictEqual(caps.canBan, false);
  assert.strictEqual(caps.canDelete, false);
  assert.strictEqual(caps.canViewQueue, false);
});

// ── symmetry ──────────────────────────────────────────────────────────────────

test("moderationCapabilities: owner and admin return the same set", () => {
  const owner = moderationCapabilities("owner");
  const admin = moderationCapabilities("admin");
  assert.deepEqual(owner, admin);
});

test("moderationCapabilities: moderator is a strict subset of admin capabilities", () => {
  const admin = moderationCapabilities("admin");
  const mod = moderationCapabilities("moderator");
  // Every capability a moderator holds, admin also holds.
  for (const key of Object.keys(mod)) {
    if (mod[key]) {
      assert.strictEqual(admin[key], true, `admin must also have ${key}`);
    }
  }
  // But admin has capabilities the moderator does not.
  assert.strictEqual(admin.canBan, true);
  assert.strictEqual(mod.canBan, false);
});
