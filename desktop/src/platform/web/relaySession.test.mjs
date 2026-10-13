import assert from "node:assert/strict";
import test from "node:test";
import {
  _resetRelaySessionForTests,
  dropRelaySession,
  relaySessionAuth,
  revokeRelaySession,
} from "./relaySession.ts";

const ok = (body) => ({ ok: true, status: 200, json: async () => body });
const PK = "a".repeat(64);

test("relay session: mints once, reuses, drops on 401, falls back when unsupported", async () => {
  _resetRelaySessionForTests();
  let mints = 0;
  const deps = {
    base: "https://r",
    pubkey: PK,
    mint: async () => (
      mints++,
      ok({ token: "t1", expires_at: Math.floor(Date.now() / 1000) + 3600 })
    ),
  };
  const [a, b] = await Promise.all([
    relaySessionAuth(deps),
    relaySessionAuth(deps),
  ]);
  assert.equal(a, "BuzzSession t1");
  assert.equal(b, "BuzzSession t1");
  assert.equal(await relaySessionAuth(deps), "BuzzSession t1");
  assert.equal(mints, 1);
  dropRelaySession("BuzzSession t1");
  await relaySessionAuth(deps);
  assert.equal(mints, 2);

  _resetRelaySessionForTests();
  const old = {
    base: "https://old",
    pubkey: PK,
    mint: async () => ({ ok: false, status: 404, json: async () => ({}) }),
  };
  assert.equal(await relaySessionAuth(old), null);
  assert.equal(
    await relaySessionAuth({
      ...old,
      mint: async () => assert.fail("backed off"),
    }),
    null,
  );
  assert.equal(await relaySessionAuth({ ...deps, pubkey: null }), null);
});

test("relay session: revoke forgets the token and calls DELETE", async () => {
  _resetRelaySessionForTests();
  const deps = {
    base: "https://r",
    pubkey: PK,
    mint: async () =>
      ok({ token: "t9", expires_at: Math.floor(Date.now() / 1000) + 3600 }),
  };
  await relaySessionAuth(deps);
  const calls = [];
  revokeRelaySession(async (base, auth) => calls.push([base, auth]));
  assert.deepEqual(calls, [["https://r", "BuzzSession t9"]]);
  let minted = false;
  await relaySessionAuth({
    ...deps,
    mint: async () => (
      (minted = true),
      ok({ token: "t10", expires_at: Math.floor(Date.now() / 1000) + 3600 })
    ),
  });
  assert.equal(minted, true);
});
