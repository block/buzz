import assert from "node:assert/strict";
import test from "node:test";

import {
  clearCommunityOnboardingTransaction,
  isTransactionStillConnecting,
  loadCommunityOnboardingTransaction,
  markCommunityOnboardingComplete,
  resolveProfileCheckAction,
  shouldSkipCommunityOnboarding,
  startCommunityOnboarding,
  updateCommunityOnboardingTransaction,
  updateCurrentCommunityOnboardingTransaction,
} from "./communityOnboarding.tsx";

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  };
}

test("invite onboarding starts at claim and normalizes its relay", () => {
  const storage = createMemoryStorage();
  const transaction = startCommunityOnboarding(
    {
      source: "deep-link-join",
      relayUrl: "WSS://Relay.Example/path/",
      inviteCode: "  invite-code  ",
    },
    storage,
    new Date("2026-07-16T00:00:00Z"),
  );
  assert.equal(transaction.stage, "claiming");
  assert.equal(transaction.relayUrl, "wss://relay.example/path");
  assert.equal(transaction.inviteCode, "invite-code");
  const persisted = loadCommunityOnboardingTransaction(storage);
  assert.equal(persisted?.id, transaction.id);
  assert.equal(persisted?.stage, transaction.stage);
  assert.equal(persisted?.relayUrl, transaction.relayUrl);
});

test("non-invite onboarding starts at connection", () => {
  const transaction = startCommunityOnboarding(
    { source: "add-community", relayUrl: "wss://relay.example" },
    createMemoryStorage(),
  );
  assert.equal(transaction.stage, "connecting");
});

test("same-relay ingress resumes rather than replacing progress", () => {
  const storage = createMemoryStorage();
  const first = startCommunityOnboarding(
    { source: "add-community", relayUrl: "wss://relay.example" },
    storage,
    new Date("2026-07-16T00:00:00Z"),
  );
  const progressed = updateCommunityOnboardingTransaction(
    first,
    { stage: "profile", communityId: "community-id" },
    storage,
    new Date("2026-07-16T00:01:00Z"),
  );
  const resumed = startCommunityOnboarding(
    {
      source: "deep-link-join",
      relayUrl: "wss://relay.example/",
      inviteCode: "new-code",
    },
    storage,
    new Date("2026-07-16T00:02:00Z"),
  );
  assert.equal(resumed.id, progressed.id);
  assert.equal(resumed.stage, "profile");
  assert.equal(resumed.communityId, "community-id");
  assert.equal(resumed.inviteCode, "new-code");
});

test("stale asynchronous updates cannot mutate a replacement transaction", () => {
  const storage = createMemoryStorage();
  const original = startCommunityOnboarding(
    {
      source: "deep-link-join",
      relayUrl: "wss://relay.example",
      inviteCode: "invite-code",
    },
    storage,
  );
  const replacement = startCommunityOnboarding(
    { source: "deep-link-connect", relayUrl: "wss://other.example" },
    storage,
  );

  const result = updateCurrentCommunityOnboardingTransaction(
    replacement,
    { stage: "connecting", error: "stale error" },
    original.id,
    storage,
  );

  assert.equal(result, replacement);
  assert.equal(loadCommunityOnboardingTransaction(storage)?.id, replacement.id);
  assert.equal(loadCommunityOnboardingTransaction(storage)?.error, undefined);
});

test("acknowledgment persists but resets when the same-relay link reopens", () => {
  const storage = createMemoryStorage();
  const transaction = startCommunityOnboarding(
    { source: "deep-link-connect", relayUrl: "wss://relay.example" },
    storage,
  );
  assert.equal(transaction.stage, "connecting");
  updateCommunityOnboardingTransaction(
    transaction,
    { acknowledged: true },
    storage,
  );
  assert.equal(loadCommunityOnboardingTransaction(storage)?.acknowledged, true);
  const reopened = startCommunityOnboarding(
    { source: "deep-link-connect", relayUrl: "wss://relay.example" },
    storage,
  );
  assert.equal(reopened.acknowledged, undefined);
});

test("malformed persisted state is ignored and can be cleared", () => {
  const storage = createMemoryStorage({
    "buzz-community-onboarding-transaction.v1": '{"stage":"profile"}',
  });
  assert.equal(loadCommunityOnboardingTransaction(storage), null);
  clearCommunityOnboardingTransaction(storage);
  assert.equal(storage.length, 0);
});

test("completion is scoped by relay and pubkey and preserves legacy gate", () => {
  const storage = createMemoryStorage();
  markCommunityOnboardingComplete("pubkey", "wss://relay.example", storage);
  assert.equal(
    storage.getItem(
      "buzz-community-onboarding-complete.v1:wss%3A%2F%2Frelay.example:pubkey",
    ),
    "true",
  );
  assert.equal(storage.getItem("buzz-onboarding-complete.v1:pubkey"), "true");
});

// ── shouldSkipCommunityOnboarding ────────────────────────────────────────────

/** Minimal Profile stub — only the fields the helper inspects. */
function makeProfile(hasProfileEvent, overrides = {}) {
  return {
    pubkey: "aabbcc",
    displayName: null,
    avatarUrl: null,
    about: null,
    nip05Handle: null,
    ownerPubkey: null,
    hasProfileEvent,
    ...overrides,
  };
}

test("shouldSkipCommunityOnboarding_hasProfileEvent_returnsTrue", () => {
  assert.equal(
    shouldSkipCommunityOnboarding(makeProfile(true)),
    true,
    "existing kind:0 ⇒ skip onboarding",
  );
});

test("shouldSkipCommunityOnboarding_noProfileEvent_returnsFalse", () => {
  assert.equal(
    shouldSkipCommunityOnboarding(makeProfile(false)),
    false,
    "no kind:0 ⇒ show profile step",
  );
});

test("shouldSkipCommunityOnboarding_fetchError_returnsFalse", () => {
  // fetch error is represented as null — must never block onboarding
  assert.equal(
    shouldSkipCommunityOnboarding(null),
    false,
    "fetch error ⇒ show profile step (safe fallback)",
  );
});

// ── resolveProfileCheckAction — async orchestration ──────────────────────────

/**
 * Returns a fake scheduleTimeout that captures registered callbacks so tests
 * can fire or skip them manually without real timers.
 */
function makeScheduler() {
  const callbacks = [];
  return {
    schedule: (fn, _ms) => callbacks.push(fn),
    fireTimeout: () => {
      const fn = callbacks.shift();
      if (fn) fn();
    },
    pendingCount: () => callbacks.length,
  };
}

test("resolveProfileCheckAction_hasProfileEvent_returnsSkipWithProfile", async () => {
  const profile = makeProfile(true, { pubkey: "aabbcc" });
  const result = await resolveProfileCheckAction(
    () => Promise.resolve(profile),
    10_000,
    makeScheduler().schedule,
  );
  assert.equal(result.action, "skip");
  assert.equal(result.profile.pubkey, "aabbcc");
});

test("resolveProfileCheckAction_noProfileEvent_returnsShowProfile", async () => {
  const result = await resolveProfileCheckAction(
    () => Promise.resolve(makeProfile(false)),
    10_000,
    makeScheduler().schedule,
  );
  assert.equal(result.action, "show-profile");
});

test("resolveProfileCheckAction_fetchRejects_returnsShowProfile", async () => {
  const result = await resolveProfileCheckAction(
    () => Promise.reject(new Error("network error")),
    10_000,
    makeScheduler().schedule,
  );
  assert.equal(result.action, "show-profile");
});

test("resolveProfileCheckAction_timeout_returnsShowProfile", async () => {
  // Fetch never settles; scheduler fires the timeout immediately.
  const scheduler = makeScheduler();
  const result = await resolveProfileCheckAction(
    () => new Promise(() => {}), // hangs forever
    10_000,
    (fn, ms) => {
      scheduler.schedule(fn, ms);
      // Fire synchronously so the test does not wait for a real timer.
      scheduler.fireTimeout();
    },
  );
  assert.equal(
    result.action,
    "show-profile",
    "timeout ⇒ show-profile (never strands onboarding)",
  );
});

test("resolveProfileCheckAction_lateSuccessAfterTimeout_doesNotSkip", async () => {
  // Fetch resolves AFTER the timeout has already fired.
  // resolveProfileCheckAction must return show-profile from the timeout path,
  // and the late fetch result must have no effect.
  let resolveFetch;
  const fetchPromise = new Promise((resolve) => {
    resolveFetch = resolve;
  });

  const scheduler = makeScheduler();
  const resultPromise = resolveProfileCheckAction(
    () => fetchPromise,
    10_000,
    scheduler.schedule,
  );

  // Fire the timeout — race settles with the timeout rejection.
  scheduler.fireTimeout();

  // Now resolve the fetch with a kind:0 profile.
  resolveFetch(makeProfile(true));

  const result = await resultPromise;
  assert.equal(
    result.action,
    "show-profile",
    "late success after timeout must not complete onboarding",
  );
});

// ── isTransactionStillConnecting — stale-transaction guard ───────────────────

/**
 * Builds a minimal transaction stub for testing the guard predicate.
 * Only `id` and `stage` are inspected by isTransactionStillConnecting.
 */
function makeTransaction(id, stage) {
  return {
    id,
    stage,
    source: "add-community",
    relayUrl: "wss://relay.example",
    communityName: "test",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}

test("isTransactionStillConnecting_matchingIdAndStage_returnsTrue", () => {
  assert.equal(
    isTransactionStillConnecting(makeTransaction("tx-a", "connecting"), "tx-a"),
    true,
    "same id + connecting stage → guard passes",
  );
});

test("isTransactionStillConnecting_replacedTransaction_returnsFalse", () => {
  // Transaction B replaced A while the fetch was in flight.
  // The callback for A must not clear B.
  assert.equal(
    isTransactionStillConnecting(makeTransaction("tx-b", "connecting"), "tx-a"),
    false,
    "different id (replacement) → guard rejects stale success",
  );
});

test("isTransactionStillConnecting_cancelWithNoReplacement_returnsFalse", () => {
  // User cancelled without starting a new transaction; ref is null.
  assert.equal(
    isTransactionStillConnecting(null, "tx-a"),
    false,
    "null ref (cancel without replacement) → guard rejects stale success",
  );
});

test("isTransactionStillConnecting_stagePastConnecting_returnsFalse", () => {
  // Fallback already advanced the transaction to 'profile' before the skip
  // result arrived — double-completion must not occur.
  assert.equal(
    isTransactionStillConnecting(makeTransaction("tx-a", "profile"), "tx-a"),
    false,
    "stage advanced past connecting → guard rejects late action",
  );
});
