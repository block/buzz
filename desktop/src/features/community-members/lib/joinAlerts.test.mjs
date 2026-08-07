import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_JOIN_ALERT_LEDGER,
  JOIN_ALERT_DEPARTED_MAX_ITEMS,
  joinAlertBody,
  joinAlertTitle,
  readJoinAlertLedger,
  reconcileJoinAlertLedger,
  writeJoinAlertLedger,
} from "./joinAlerts.ts";

const COMMUNITY = "community-1";
const OWNER = "a".repeat(64);
const ALICE = "b".repeat(64);
const BOB = "c".repeat(64);

function installLocalStorage({ throwOnSet = false } = {}) {
  const values = new Map();
  globalThis.window = {
    localStorage: {
      get length() {
        return values.size;
      },
      key: (index) => [...values.keys()][index] ?? null,
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => {
        if (throwOnSet) {
          const error = new Error("quota exceeded");
          error.name = "QuotaExceededError";
          throw error;
        }
        values.set(key, value);
      },
      removeItem: (key) => values.delete(key),
    },
  };
  return values;
}

/** Fold a roster in and persist, the way the hook does. */
function applySnapshot(ledger, rosterPubkeys) {
  const result = reconcileJoinAlertLedger({
    ledger,
    rosterPubkeys,
    viewerPubkey: OWNER,
  });
  if (result.changed) {
    writeJoinAlertLedger(COMMUNITY, OWNER, result.ledger);
  }
  return result;
}

test("first snapshot seeds an existing roster without alerting", () => {
  installLocalStorage();

  const result = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER, ALICE, BOB]);

  assert.deepEqual(result.alerts, []);
  assert.equal(result.ledger.seeded, true);
  assert.deepEqual(result.ledger.pubkeys, [ALICE, BOB]);
});

test("a key joining after the seed alerts exactly once", () => {
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER, ALICE]).ledger;
  const joined = applySnapshot(seeded, [OWNER, ALICE, BOB]);

  assert.deepEqual(joined.alerts, [BOB]);

  // A redelivered identical snapshot must not re-alert or rewrite.
  const redelivered = applySnapshot(joined.ledger, [OWNER, ALICE, BOB]);
  assert.deepEqual(redelivered.alerts, []);
  assert.equal(redelivered.changed, false);
});

test("a community seeded with only the viewer still alerts on the first join", () => {
  // Regression: inferring "seeded" from a non-empty ledger classified this
  // first genuine join as the seeding run and dropped the alert silently.
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER]);
  assert.deepEqual(seeded.alerts, []);
  assert.deepEqual(seeded.ledger.pubkeys, []);
  assert.equal(seeded.ledger.seeded, true);

  const joined = applySnapshot(seeded.ledger, [OWNER, ALICE]);
  assert.deepEqual(joined.alerts, [ALICE]);
});

test("the seeded flag survives a reload through storage", () => {
  installLocalStorage();

  applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER]);
  const reloaded = readJoinAlertLedger(COMMUNITY, OWNER);

  assert.equal(reloaded.seeded, true);
  assert.deepEqual(reloaded.pubkeys, []);
  assert.deepEqual(applySnapshot(reloaded, [OWNER, ALICE]).alerts, [ALICE]);
});

test("remove then re-add does not alert a second time", () => {
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER]).ledger;
  assert.deepEqual(applySnapshot(seeded, [OWNER, ALICE]).alerts, [ALICE]);

  const afterRemoval = applySnapshot(readJoinAlertLedger(COMMUNITY, OWNER), [
    OWNER,
  ]);
  assert.deepEqual(afterRemoval.alerts, []);

  const afterReAdd = applySnapshot(readJoinAlertLedger(COMMUNITY, OWNER), [
    OWNER,
    ALICE,
  ]);
  assert.deepEqual(afterReAdd.alerts, []);
});

test("the kind:8000 accelerator and the live snapshot yield one alert", () => {
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER]).ledger;

  // Delta arrives first and triggers a snapshot refetch...
  const viaDelta = applySnapshot(seeded, [OWNER, ALICE]);
  assert.deepEqual(viaDelta.alerts, [ALICE]);

  // ...then the live 13534 for the same join lands.
  const viaLive = applySnapshot(readJoinAlertLedger(COMMUNITY, OWNER), [
    OWNER,
    ALICE,
  ]);
  assert.deepEqual(viaLive.alerts, []);
});

test("the viewer is never alerted on or recorded", () => {
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [ALICE]).ledger;
  const result = applySnapshot(seeded, [ALICE, OWNER]);

  assert.deepEqual(result.alerts, []);
  assert.equal(result.changed, false);
  assert.equal(result.ledger.pubkeys.includes(OWNER), false);
});

test("roster pubkeys are matched case-insensitively", () => {
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER]).ledger;
  const joined = applySnapshot(seeded, [OWNER, ALICE.toUpperCase()]);

  assert.deepEqual(joined.alerts, [ALICE]);
  assert.deepEqual(applySnapshot(joined.ledger, [OWNER, ALICE]).alerts, []);
});

test("a duplicated pubkey in one snapshot alerts once", () => {
  installLocalStorage();

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, [OWNER]).ledger;
  const joined = applySnapshot(seeded, [OWNER, ALICE, ALICE]);

  assert.deepEqual(joined.alerts, [ALICE]);
  assert.deepEqual(joined.ledger.pubkeys, [ALICE]);
});

test("a ledger stored before the seeded flag existed is treated as seeded", () => {
  const values = installLocalStorage();
  const [key] = [...values.keys()];
  writeJoinAlertLedger(COMMUNITY, OWNER, { seeded: true, pubkeys: [ALICE] });
  const storageKey = key ?? [...values.keys()][0];
  values.set(storageKey, JSON.stringify({ pubkeys: [ALICE] }));

  const ledger = readJoinAlertLedger(COMMUNITY, OWNER);
  assert.equal(ledger.seeded, true);
  assert.deepEqual(applySnapshot(ledger, [OWNER, ALICE, BOB]).alerts, [BOB]);
});

test("unreadable storage reads as an unseeded ledger", () => {
  const values = installLocalStorage();
  writeJoinAlertLedger(COMMUNITY, OWNER, { seeded: true, pubkeys: [ALICE] });
  values.set([...values.keys()][0], "{not json");

  assert.deepEqual(readJoinAlertLedger(COMMUNITY, OWNER), {
    seeded: false,
    pubkeys: [],
  });
});

test("a roster larger than the departed cap never re-alerts its own members", () => {
  // Regression: capping *all* retained keys shed pubkeys that were still on the
  // roster, so the next snapshot saw them as unknown and alerted again — every
  // snapshot, forever, for any community past the cap.
  installLocalStorage();

  const roster = Array.from(
    { length: JOIN_ALERT_DEPARTED_MAX_ITEMS + 100 },
    (_unused, index) => index.toString(16).padStart(64, "0"),
  );

  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, roster);
  assert.deepEqual(seeded.alerts, []);
  assert.equal(seeded.ledger.pubkeys.length, roster.length);

  for (let pass = 0; pass < 3; pass++) {
    const repeat = applySnapshot(readJoinAlertLedger(COMMUNITY, OWNER), roster);
    assert.deepEqual(repeat.alerts, []);
    assert.equal(repeat.changed, false);
  }

  // The read path must not truncate either: a stored ledger above the cap has
  // to come back whole or the same re-alert loop reopens on reload.
  assert.equal(
    readJoinAlertLedger(COMMUNITY, OWNER).pubkeys.length,
    roster.length,
  );
});

test("the cap sheds only departed pubkeys, oldest first", () => {
  installLocalStorage();

  const roster = Array.from(
    { length: JOIN_ALERT_DEPARTED_MAX_ITEMS + 10 },
    (_unused, index) => index.toString(16).padStart(64, "0"),
  );
  const seeded = applySnapshot(EMPTY_JOIN_ALERT_LEDGER, roster).ledger;

  // Everyone leaves except the newest member; one new key joins.
  const survivor = roster.at(-1);
  const shrunk = applySnapshot(seeded, [OWNER, survivor, BOB]);

  assert.deepEqual(shrunk.alerts, [BOB]);
  // 5010 retained - 9 departed over the cap, plus BOB.
  assert.equal(shrunk.ledger.pubkeys.length, roster.length - 9 + 1);
  assert.equal(shrunk.ledger.pubkeys.includes(roster[0]), false);
  assert.equal(shrunk.ledger.pubkeys.includes(roster[8]), false);
  assert.equal(shrunk.ledger.pubkeys.includes(roster[9]), true);
  // The on-roster key is retained no matter where it sits in insertion order.
  assert.equal(shrunk.ledger.pubkeys.includes(survivor), true);
});

test("a write that cannot land is reported, not thrown", () => {
  // The writer runs inside an async snapshot handler: a raw QuotaExceededError
  // would reject before the notification is sent, on every snapshot.
  installLocalStorage({ throwOnSet: true });

  assert.equal(
    writeJoinAlertLedger(COMMUNITY, OWNER, { seeded: true, pubkeys: [ALICE] }),
    false,
  );
  assert.deepEqual(readJoinAlertLedger(COMMUNITY, OWNER), {
    seeded: false,
    pubkeys: [],
  });
});

test("notification copy names the community when known", () => {
  assert.equal(joinAlertTitle("Buzz HQ"), "New member in Buzz HQ");
  assert.equal(joinAlertTitle("  "), "New community member");
  assert.equal(joinAlertTitle(null), "New community member");
  assert.equal(joinAlertBody("Alice"), "Alice joined");
});
