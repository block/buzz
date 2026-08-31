import assert from "node:assert/strict";
import test from "node:test";
import { fetchHostHistory } from "./history.ts";
import { fixture } from "./hostTestFixtures.mjs";

const filter = { kinds: [50000], limit: 1000 };
const event = (id, created_at = 100) => ({
  id: id.toString(16).padStart(64, "0"),
  created_at,
});

test("keyset exhausts more than two full pages of same-second events", async () => {
  const rows = Array.from({ length: 2501 }, (_, i) => event(i + 1));
  const calls = [];
  const result = await fetchHostHistory(
    async (f) => {
      calls.push(f);
      return rows
        .filter((e) => !f.before_id || e.id > f.before_id)
        .slice(0, f.limit)
        .reverse();
    },
    filter,
    () => {},
  );
  assert.deepEqual(result, rows);
  assert.equal(calls.length, 3);
  assert.equal(calls[1].until, 100);
  assert.equal(calls[1].before_id, rows[999].id);
});

test("exact full page needs an empty successor, not an incomplete-history error", async () => {
  let calls = 0;
  const result = await fetchHostHistory(
    async (f) => {
      calls++;
      return f.before_id
        ? []
        : Array.from({ length: 1000 }, (_, i) => event(i + 1));
    },
    filter,
    () => {},
  );
  assert.equal(result.length, 1000);
  assert.equal(calls, 2);
});

test("old server ignoring the cursor fails closed rather than loops or suppresses a write", async () => {
  const page = Array.from({ length: 1000 }, (_, i) => event(i + 1));
  await assert.rejects(
    fetchHostHistory(
      async () => [...page],
      filter,
      () => {},
    ),
    /cursor/,
  );
});

test("failure or cancellation on a later page discards the partial history", async () => {
  for (const cancel of [false, true]) {
    let active = true;
    let calls = 0;
    await assert.rejects(
      fetchHostHistory(
        async () => {
          calls++;
          if (calls === 2) {
            if (!cancel) throw new Error("offline");
            active = false;
            return [];
          }
          return Array.from({ length: 1000 }, (_, i) => event(i + 1));
        },
        filter,
        () => {
          if (!active) throw new Error("cancelled");
        },
      ),
      /offline|cancelled/,
    );
    assert.equal(calls, 2);
  }
});

test("registration and profile reconciliation both exceed 1000, preserving newest valid profile", async () => {
  const f = fixture();
  await f.run();
  const registration = f.events[0];
  const profile = f.events[1];
  // All bindings target this host, as can happen after repeated older-client
  // registration attempts. Newest profiles include unreadable ciphertext.
  for (let i = 1; i <= 1100; i++) {
    f.events.push({ ...registration, id: event(i + 500000).id });
    f.events.push({
      ...profile,
      id: event(i + 600000).id,
      created_at: 101,
      decoded: undefined,
    });
  }
  f.setNow(103);
  f.writes.length = 0;
  const result = await f.run();
  assert.equal(result.rows.length, 1);
  assert.equal(result.rows[0].event.id, profile.id);
  assert.equal(f.writes.length, 0);
});
