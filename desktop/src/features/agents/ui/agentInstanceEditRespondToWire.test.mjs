import assert from "node:assert/strict";
import test from "node:test";

import { computeRespondToWirePatch } from "./personaRuntimeModel.ts";

// ── Submit-shape wire contract for the agent-instance edit dialog ────────────
//
// Issue #2501: the Desktop edit dialog's allowlist wire condition conflates
// "mode changed" with "payload changed", so several mode/allowlist transition
// shapes silently drop the payload the harness layer actually needs. The
// harness reads from `respond_to` + `respond_to_allowlist` on the local record
// ( not the definition snapshot projection), and `update_managed_agent` merges
// `undefined` allowlist as "leave unchanged" — so a mode flip WITHOUT a
// re-sent payload can leave the record in an unreachable or crash-looping
// state. These tests pin the exact wire patch each transition must produce.

const PUBKEY_A = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2";
const PUBKEY_B = "b1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2";

test("owner-only → allowlist with fresh list sends mode AND allowlist", () => {
  // The reporter's primary flow: pick Allowlist, add a pubkey, Save.
  const patch = computeRespondToWirePatch({
    currentMode: "owner-only",
    currentAllowlist: [],
    submitMode: "allowlist",
    submitAllowlist: [PUBKEY_A],
  });
  assert.equal(patch.respondTo, "allowlist");
  assert.deepEqual(patch.respondToAllowlist, [PUBKEY_A]);
});

test("allowlist → allowlist editing the list sends updated allowlist", () => {
  // Mode unchanged, list edited — the payload must go on the wire.
  const patch = computeRespondToWirePatch({
    currentMode: "allowlist",
    currentAllowlist: [PUBKEY_A],
    submitMode: "allowlist",
    submitAllowlist: [PUBKEY_B],
  });
  assert.equal(patch.respondTo, undefined);
  assert.deepEqual(patch.respondToAllowlist, [PUBKEY_B]);
});

test("allowlist → anyone omits allowlist (record keeps stale list by design)", () => {
  // Flipping away from allowlist: the Rust update preserves the persisted
  // list across mode toggles, so we don't send it — but we must send the mode.
  const patch = computeRespondToWirePatch({
    currentMode: "allowlist",
    currentAllowlist: [PUBKEY_A],
    submitMode: "anyone",
    submitAllowlist: [PUBKEY_A],
  });
  assert.equal(patch.respondTo, "anyone");
  assert.equal(patch.respondToAllowlist, undefined);
});

test("owner-only → owner-only (no-op) sends nothing", () => {
  const patch = computeRespondToWirePatch({
    currentMode: "owner-only",
    currentAllowlist: [],
    submitMode: "owner-only",
    submitAllowlist: [],
  });
  assert.equal(patch.respondTo, undefined);
  assert.equal(patch.respondToAllowlist, undefined);
});

test("anyone → allowlist with pre-existing list sends mode AND list", () => {
  // Mode flip to allowlist where the list was already populated by an earlier
  // session (the "Anyone → Allowlist, list already there" case): the payload
  // must be re-sent so the harness applies the full {mode, list} atomically.
  const patch = computeRespondToWirePatch({
    currentMode: "anyone",
    currentAllowlist: [PUBKEY_A],
    submitMode: "allowlist",
    submitAllowlist: [PUBKEY_A],
  });
  assert.equal(patch.respondTo, "allowlist");
  assert.deepEqual(patch.respondToAllowlist, [PUBKEY_A]);
});

test("allowlist → owner-only omits allowlist", () => {
  // Flipping away: same as the allowlist → anyone case — mode only.
  const patch = computeRespondToWirePatch({
    currentMode: "allowlist",
    currentAllowlist: [PUBKEY_A],
    submitMode: "owner-only",
    submitAllowlist: [PUBKEY_A],
  });
  assert.equal(patch.respondTo, "owner-only");
  assert.equal(patch.respondToAllowlist, undefined);
});
