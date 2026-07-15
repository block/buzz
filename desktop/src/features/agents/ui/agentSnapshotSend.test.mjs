import assert from "node:assert/strict";
import test from "node:test";

// Tests for the active snapshot send controller used by PersonaShareDialog.

import {
  isSendableDestination,
  createSendGuard,
  runSendPipeline,
  runGuardedSend,
  checkSendEligibility,
} from "./useSnapshotSendController.ts";

import {
  recordTimeoutFromRejection,
  clearTimeoutState,
} from "../../moderation/lib/timeoutStore.ts";
import { buildOutgoingMessage } from "../../messages/lib/imetaMediaMarkdown.ts";

// ── isSendableDestination ─────────────────────────────────────────────────────

function makeChannel(overrides = {}) {
  return {
    id: "ch-1",
    name: "general",
    channelType: "stream",
    visibility: "public",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("isSendableDestination_stream_member_not_archived_returns_true", () => {
  const ch = makeChannel({
    channelType: "stream",
    isMember: true,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), true);
});

test("isSendableDestination_dm_member_not_archived_returns_true", () => {
  const ch = makeChannel({
    channelType: "dm",
    isMember: true,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), true);
});

test("isSendableDestination_forum_is_excluded", () => {
  const ch = makeChannel({
    channelType: "forum",
    isMember: true,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), false);
});

test("isSendableDestination_non_member_is_excluded", () => {
  const ch = makeChannel({
    channelType: "stream",
    isMember: false,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), false);
});

test("isSendableDestination_archived_is_excluded", () => {
  const ch = makeChannel({
    channelType: "stream",
    isMember: true,
    archivedAt: "2025-01-01T00:00:00Z",
  });
  assert.equal(isSendableDestination(ch), false);
});

test("isSendableDestination_archived_dm_is_excluded", () => {
  const ch = makeChannel({
    channelType: "dm",
    isMember: true,
    archivedAt: "2025-01-01T00:00:00Z",
  });
  assert.equal(isSendableDestination(ch), false);
});

// ── createSendGuard: production concurrency guard ─────────────────────────────
//
// The UI hides the confirm button the moment handleSend transitions to the
// "progress" step.  The DOM-level double-click guard is therefore the step
// transition.  createSendGuard protects against a programmatic double-invocation
// covering the entire prepare → encode → upload → send sequence.
// This test imports and exercises the production guard factory directly.

test("createSendGuard_blocks_second_concurrent_action", async () => {
  const guard = createSendGuard();
  let callCount = 0;
  const callOrder = [];

  async function action() {
    callCount++;
    callOrder.push("start");
    // Simulate async work (prepare + encode + upload + send).
    await new Promise((resolve) => setTimeout(resolve, 20));
    callOrder.push("end");
    return true;
  }

  // Fire both concurrently — the second sees inFlight=true and returns false.
  const [r1, r2] = await Promise.all([
    guard.runGuarded(action),
    guard.runGuarded(action),
  ]);

  // Exactly one invocation ran.
  assert.equal(callCount, 1, `expected callCount=1, got ${callCount}`);
  // One returned true (ran), one returned false (blocked).
  const successes = [r1, r2].filter(Boolean).length;
  assert.equal(successes, 1, `expected 1 success, got ${successes}`);
  // The single run completed fully (start then end, no interleaving).
  assert.deepEqual(callOrder, ["start", "end"]);
  // Guard is idle after both settle.
  assert.equal(
    guard.inFlight,
    false,
    "guard should be idle after both calls settle",
  );
});

test("createSendGuard_sequential_calls_both_run", async () => {
  const guard = createSendGuard();
  let count = 0;
  const run = () =>
    guard.runGuarded(async () => {
      count++;
      return true;
    });
  // Sequential calls (await each before starting next) both succeed.
  assert.equal(await run(), true);
  assert.equal(await run(), true);
  assert.equal(count, 2);
});

// ── runGuardedSend: production composition of guard + pipeline ────────────────
//
// runGuardedSend is the exact production composition that beginSend uses.
// Calling it twice concurrently with the same guard must produce exactly one
// encode, one upload, one send, and one blocked call.
// A test that stays green after encode is moved outside the guard or after
// beginSend stops calling runGuardedSend is not a production-composition test;
// this test is — it will fail in both those cases.

test("runGuardedSend_concurrent_calls_one_encodes_one_blocked", async () => {
  const guard = createSendGuard();
  let destinationCount = 0;
  let encodeCount = 0;
  let uploadCount = 0;
  let sendCount = 0;
  const states = [];

  const resolveChannelId = async () => {
    destinationCount++;
    await new Promise((resolve) => setTimeout(resolve, 5));
    return "ch-1";
  };
  const makeDeps = (channelId) => ({
    channelId,
    checkEligibilityFn: () => null,
    encodeFn: async () => {
      encodeCount++;
      // Simulate encode latency so the second call definitely arrives
      // while the first is in-flight.
      await new Promise((resolve) => setTimeout(resolve, 20));
      return { fileBytes: [1], fileName: "x.json" };
    },
    uploadFn: async (_bytes, _filename) => {
      uploadCount++;
      return {
        url: "https://example.com/x.json",
        sha256: "a".repeat(64),
        size: 1,
        type: "application/json",
        uploaded: 0,
      };
    },
    sendFn: async () => {
      sendCount++;
    },
    setStateFn: (s) => states.push(s.phase),
    buildMessageFn: (_d) => ({ content: "", mediaTags: null }),
  });

  // Fire both concurrently using the same guard.
  const [r1, r2] = await Promise.all([
    runGuardedSend(guard, resolveChannelId, makeDeps, (s) =>
      states.push(s.phase),
    ),
    runGuardedSend(guard, resolveChannelId, makeDeps, (s) =>
      states.push(s.phase),
    ),
  ]);

  // Exactly one destination open, encode, upload, and send ran.
  assert.equal(
    destinationCount,
    1,
    `expected destinationCount=1, got ${destinationCount}`,
  );
  assert.equal(encodeCount, 1, `expected encodeCount=1, got ${encodeCount}`);
  assert.equal(uploadCount, 1, `expected uploadCount=1, got ${uploadCount}`);
  assert.equal(sendCount, 1, `expected sendCount=1, got ${sendCount}`);
  // One succeeded, one was blocked.
  const successes = [r1, r2].filter(Boolean).length;
  assert.equal(successes, 1, `expected 1 success, got ${successes}`);
  // Guard is idle after both settle.
  assert.equal(guard.inFlight, false, "guard should be idle after both settle");
});

// ── runSendPipeline: production pipeline with injected deps ──────────────────
//
// runSendPipeline is the actual production function called by the hook's
// beginSend.  Tests inject mock deps and call it directly so they remain load-
// bearing: removing either checkEligibilityFn call from the production function
// breaks these tests.

test("runSendPipeline_checkpoint1_blocks_encode_upload_send", async () => {
  // checkEligibilityFn returns an error string at checkpoint 1 (before encode).
  // encode, upload, and send must not run.
  let encodeCount = 0;
  let uploadCount = 0;
  let sendCount = 0;
  const states = [];

  const result = await runSendPipeline({
    channelId: "ch-1",
    checkEligibilityFn: () => "destination archived",
    encodeFn: async () => {
      encodeCount++;
      return { fileBytes: [1], fileName: "x.json" };
    },
    uploadFn: async (_bytes, _filename) => {
      uploadCount++;
      return {
        url: "https://example.com/x.json",
        sha256: "a".repeat(64),
        size: 1,
        type: "application/json",
        uploaded: 0,
      };
    },
    sendFn: async () => {
      sendCount++;
    },
    setStateFn: (s) => states.push(s.phase),
    buildMessageFn: (d) => ({ content: "", mediaTags: [[d.url ?? ""]] }),
  });

  assert.equal(result, false, "expected pipeline to return false");
  assert.equal(encodeCount, 0, "encode must not run when checkpoint 1 fails");
  assert.equal(uploadCount, 0, "upload must not run when checkpoint 1 fails");
  assert.equal(sendCount, 0, "send must not run when checkpoint 1 fails");
  // State must be set to error (not preparing/uploading/sending).
  assert.ok(
    states.includes("error"),
    `expected error state, got ${JSON.stringify(states)}`,
  );
  assert.ok(!states.includes("preparing"), "must not reach preparing");
  assert.ok(!states.includes("uploading"), "must not reach uploading");
});

test("runSendPipeline_checkpoint2_blocks_upload_after_encode", async () => {
  // checkEligibilityFn passes at checkpoint 1, then returns an error at
  // checkpoint 2 (after encode completes).  Encode must run once; upload and
  // send must not run.
  let encodeCount = 0;
  let uploadCount = 0;
  let sendCount = 0;
  const states = [];
  let encodeComplete = false;

  const result = await runSendPipeline({
    channelId: "ch-1",
    checkEligibilityFn: () => {
      // checkpoint 1: passes; checkpoint 2: fails (called after encode)
      if (!encodeComplete) return null;
      return "channel became forum during encode";
    },
    encodeFn: async () => {
      encodeCount++;
      await new Promise((resolve) => setTimeout(resolve, 5));
      encodeComplete = true;
      return { fileBytes: [1], fileName: "x.json" };
    },
    uploadFn: async (_bytes, _filename) => {
      uploadCount++;
      return {
        url: "https://example.com/x.json",
        sha256: "a".repeat(64),
        size: 1,
        type: "application/json",
        uploaded: 0,
      };
    },
    sendFn: async () => {
      sendCount++;
    },
    setStateFn: (s) => states.push(s.phase),
    buildMessageFn: (d) => ({ content: "", mediaTags: [[d.url ?? ""]] }),
  });

  assert.equal(result, false, "expected pipeline to return false");
  assert.equal(encodeCount, 1, "encode ran once (checkpoint 1 passed)");
  assert.equal(uploadCount, 0, "upload must not run when checkpoint 2 fails");
  assert.equal(sendCount, 0, "send must not run when checkpoint 2 fails");
  const seenPreparing = states.includes("preparing");
  assert.ok(seenPreparing, "pipeline must set preparing phase");
  const seenUploading = states.includes("uploading");
  assert.ok(!seenUploading, "must not reach uploading when checkpoint 2 fails");
  assert.ok(
    states.includes("error"),
    "must set error state after checkpoint 2",
  );
});

test("runSendPipeline_avatar_bearing_snapshot_omits_thumb_and_keeps_filename", async () => {
  let sentMediaTags = null;

  const result = await runSendPipeline({
    channelId: "ch-1",
    checkEligibilityFn: () => null,
    encodeFn: async () => ({
      fileBytes: [1],
      fileName: "avatar-bearing.agent.json",
    }),
    uploadFn: async () => ({
      url: "https://example.com/avatar-bearing.agent.json",
      sha256: "a".repeat(64),
      size: 1,
      type: "application/json",
      uploaded: 0,
      // This represents an upload descriptor that happens to include a thumb.
      // A snapshot send must not serialize it as NIP-92 imeta metadata.
      thumb: "https://example.com/avatar.png",
    }),
    sendFn: async ({ mediaTags }) => {
      sentMediaTags = mediaTags;
    },
    setStateFn: () => {},
    buildMessageFn: (descriptor) => buildOutgoingMessage("", [descriptor]),
  });

  assert.equal(result, true);
  assert.deepEqual(sentMediaTags, [
    [
      "imeta",
      "url https://example.com/avatar-bearing.agent.json",
      "m application/json",
      `x ${"a".repeat(64)}`,
      "size 1",
      "filename avatar-bearing.agent.json",
    ],
  ]);
  assert.ok(
    sentMediaTags[0].includes("filename avatar-bearing.agent.json"),
    "snapshot imeta must retain its filename",
  );
  assert.ok(
    !sentMediaTags[0].some((entry) => entry.startsWith("thumb ")),
    "snapshot imeta must not include a thumb field",
  );
});

test("runSendPipeline_happy_path_sets_all_phases", async () => {
  // All checkpoints pass and encode/upload/send succeed — full phase sequence.
  const states = [];
  let sendArgs = null;

  const result = await runSendPipeline({
    channelId: "ch-1",
    checkEligibilityFn: () => null,
    encodeFn: async () => ({ fileBytes: [1], fileName: "x.json" }),
    uploadFn: async (_bytes, _filename) => ({
      url: "https://example.com/x.json",
      sha256: "a".repeat(64),
      size: 1,
      type: "application/json",
      uploaded: 0,
    }),
    sendFn: async (args) => {
      sendArgs = args;
    },
    setStateFn: (s) => states.push(s.phase),
    buildMessageFn: (_d) => ({ content: "test", mediaTags: [["tag"]] }),
  });

  assert.equal(result, true, "expected pipeline to return true");
  assert.deepEqual(states, ["preparing", "uploading", "sending", "done"]);
  assert.ok(sendArgs, "send must have been called");
});

// ── checkSendEligibility: current-source validation ───────────────────────────
//
// checkSendEligibility reads from a QueryClient directly (not from rendered
// state).  Tests inject a minimal mock QueryClient so the function is testable
// without a React context.

function makeMockQueryClient(data) {
  return {
    getQueryData(key) {
      const k = JSON.stringify(key);
      return data[k] ?? undefined;
    },
    getQueryState(key) {
      const k = JSON.stringify(key);
      return data[`state:${k}`] ?? undefined;
    },
  };
}

test("checkSendEligibility_valid_stream_returns_null", () => {
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-1",
        channelType: "stream",
        isMember: true,
        archivedAt: null,
      }),
    ],
  });
  const result = checkSendEligibility(qc, "ch-1", 1000);
  assert.equal(result, null, "valid stream must be eligible");
});

test("checkSendEligibility_archived_channel_returns_error", () => {
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({ id: "ch-1", archivedAt: "2025-01-01T00:00:00Z" }),
    ],
  });
  const result = checkSendEligibility(qc, "ch-1", 1000);
  assert.notEqual(result, null, "archived channel must be ineligible");
});

test("checkSendEligibility_non_member_channel_returns_error", () => {
  const qc = makeMockQueryClient({
    '["channels"]': [makeChannel({ id: "ch-1", isMember: false })],
  });
  const result = checkSendEligibility(qc, "ch-1", 1000);
  assert.notEqual(result, null, "non-member channel must be ineligible");
});

test("checkSendEligibility_forum_channel_returns_error", () => {
  const qc = makeMockQueryClient({
    '["channels"]': [makeChannel({ id: "ch-1", channelType: "forum" })],
  });
  const result = checkSendEligibility(qc, "ch-1", 1000);
  assert.notEqual(result, null, "forum channel must be ineligible");
});

test("checkSendEligibility_missing_channel_returns_error", () => {
  const qc = makeMockQueryClient({ '["channels"]': [] });
  const result = checkSendEligibility(qc, "ch-1", 1000);
  assert.notEqual(result, null, "missing channel must be ineligible");
});

test("checkSendEligibility_active_timeout_known_expiry_returns_error", () => {
  // Activate the timeout store with a known future expiry, then assert blocked.
  // Clear the store in finally so state cannot leak to other tests.
  const qc = makeMockQueryClient({
    '["channels"]': [makeChannel({ id: "ch-1" })],
  });
  const futureExpiry = Date.now() + 60_000; // 1 minute from now
  try {
    recordTimeoutFromRejection(
      `restricted: you are timed out until ${Math.floor(futureExpiry / 1000)}`,
    );
    const result = checkSendEligibility(qc, "ch-1");
    assert.notEqual(
      result,
      null,
      "active timeout (known expiry) must be blocked",
    );
  } finally {
    clearTimeoutState();
  }
});

test("checkSendEligibility_active_timeout_unknown_expiry_returns_error", () => {
  // "restricted: you are timed out until 0" is the unknown-expiry case;
  // parseTimeoutRejection returns expiresAtMs=null, and isTimeoutActive(null)
  // returns true (fail-closed).
  const qc = makeMockQueryClient({
    '["channels"]': [makeChannel({ id: "ch-1" })],
  });
  try {
    recordTimeoutFromRejection("restricted: you are timed out until 0");
    const result = checkSendEligibility(qc, "ch-1");
    assert.notEqual(
      result,
      null,
      "active timeout (unknown expiry) must be blocked",
    );
  } finally {
    clearTimeoutState();
  }
});

test("checkSendEligibility_no_timeout_stream_returns_null", () => {
  // Baseline: no active timeout, valid stream channel → eligible.
  const qc = makeMockQueryClient({
    '["channels"]': [makeChannel({ id: "ch-1" })],
  });
  const result = checkSendEligibility(qc, "ch-1", 1000);
  assert.equal(result, null, "no timeout + valid stream → eligible");
});

test("checkSendEligibility_dm_with_loading_identity_returns_error", () => {
  // Fail-closed: if identity is still fetching, any DM is ineligible.
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm",
        channelType: "dm",
        participantPubkeys: ["aabb", "ccdd"],
      }),
    ],
    'state:["identity"]': { status: "pending", fetchStatus: "fetching" },
  });
  const result = checkSendEligibility(qc, "ch-dm", 1000);
  assert.notEqual(result, null, "DM with loading identity must be ineligible");
});

test("checkSendEligibility_dm_with_loading_relay_self_returns_error", () => {
  // Fail-closed: if relay-self is still fetching, any DM is ineligible.
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm",
        channelType: "dm",
        participantPubkeys: ["aabb", "ccdd"],
      }),
    ],
    'state:["identity"]': { status: "success", fetchStatus: "idle" },
    'state:["relaySelf"]': { status: "pending", fetchStatus: "fetching" },
  });
  const result = checkSendEligibility(qc, "ch-dm", 1000);
  assert.notEqual(
    result,
    null,
    "DM with loading relay-self must be ineligible",
  );
});

test("checkSendEligibility_classified_moderation_dm_returns_error", () => {
  // Fail-closed: a 1:1 DM whose only other participant is relaySelf is a
  // moderation DM and must be blocked.  Identity and relay-self are fully
  // loaded so the moderation classification can run.
  const RELAY_SELF = "relay000".padEnd(64, "0");
  const MY_PUBKEY = "mypubkey".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-mod-dm",
        channelType: "dm",
        participantPubkeys: [MY_PUBKEY, RELAY_SELF],
      }),
    ],
    '["identity"]': { pubkey: MY_PUBKEY, displayName: "Me" },
    '["relaySelf"]': RELAY_SELF,
    'state:["identity"]': { status: "success", fetchStatus: "idle" },
    'state:["relaySelf"]': { status: "success", fetchStatus: "idle" },
  });
  const result = checkSendEligibility(qc, "ch-mod-dm", 1000);
  assert.notEqual(result, null, "classified moderation DM must be blocked");
});

test("checkSendEligibility_ordinary_dm_is_eligible", () => {
  // A normal 1:1 DM whose other participant is NOT relaySelf must be eligible.
  const RELAY_SELF = "relay000".padEnd(64, "0");
  const MY_PUBKEY = "mypubkey".padEnd(64, "0");
  const OTHER_PUBKEY = "other000".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-ordinary-dm",
        channelType: "dm",
        participantPubkeys: [MY_PUBKEY, OTHER_PUBKEY],
      }),
    ],
    '["identity"]': { pubkey: MY_PUBKEY, displayName: "Me" },
    '["relaySelf"]': RELAY_SELF,
    'state:["identity"]': { status: "success", fetchStatus: "idle" },
    'state:["relaySelf"]': { status: "success", fetchStatus: "idle" },
  });
  const result = checkSendEligibility(qc, "ch-ordinary-dm", 1000);
  assert.equal(result, null, "ordinary DM must be eligible");
});

test("checkSendEligibility_absent_identity_blocks_dm", () => {
  // Fail-closed: if identity state is absent (never fetched — state undefined),
  // any DM must be blocked regardless of relay-self state.
  const RELAY_SELF = "relay000".padEnd(64, "0");
  const OTHER_PUBKEY = "other000".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm",
        channelType: "dm",
        participantPubkeys: [OTHER_PUBKEY, "deadbeef".padEnd(64, "0")],
      }),
    ],
    // identity state is undefined — never fetched
    '["relaySelf"]': RELAY_SELF,
    'state:["identity"]': undefined,
    'state:["relaySelf"]': { status: "success", fetchStatus: "idle" },
  });
  const result = checkSendEligibility(qc, "ch-dm", 1000);
  assert.notEqual(result, null, "absent identity state must block any DM");
});

test("checkSendEligibility_errored_identity_blocks_dm", () => {
  // Fail-closed: if identity query errored, any DM must be blocked.
  const RELAY_SELF = "relay000".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm",
        channelType: "dm",
        participantPubkeys: [
          "other000".padEnd(64, "0"),
          "deadbeef".padEnd(64, "0"),
        ],
      }),
    ],
    'state:["identity"]': { status: "error", fetchStatus: "idle" },
    '["relaySelf"]': RELAY_SELF,
    'state:["relaySelf"]': { status: "success", fetchStatus: "idle" },
  });
  const result = checkSendEligibility(qc, "ch-dm", 1000);
  assert.notEqual(result, null, "errored identity state must block any DM");
});

test("checkSendEligibility_absent_relay_self_blocks_dm", () => {
  // Fail-closed: if relay-self state is absent (never fetched), any DM blocks.
  const MY_PUBKEY = "mypubkey".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm",
        channelType: "dm",
        participantPubkeys: [MY_PUBKEY, "other000".padEnd(64, "0")],
      }),
    ],
    '["identity"]': { pubkey: MY_PUBKEY, displayName: "Me" },
    'state:["identity"]': { status: "success", fetchStatus: "idle" },
    // relay-self state is undefined — never fetched
    'state:["relaySelf"]': undefined,
  });
  const result = checkSendEligibility(qc, "ch-dm", 1000);
  assert.notEqual(result, null, "absent relay-self state must block any DM");
});

test("checkSendEligibility_errored_relay_self_blocks_dm", () => {
  // Fail-closed: if relay-self query errored, any DM blocks even though
  // relaySelf=null data might remain in cache from a prior success.
  const MY_PUBKEY = "mypubkey".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm",
        channelType: "dm",
        participantPubkeys: [MY_PUBKEY, "other000".padEnd(64, "0")],
      }),
    ],
    '["identity"]': { pubkey: MY_PUBKEY, displayName: "Me" },
    'state:["identity"]': { status: "success", fetchStatus: "idle" },
    'state:["relaySelf"]': { status: "error", fetchStatus: "idle" },
  });
  const result = checkSendEligibility(qc, "ch-dm", 1000);
  assert.notEqual(result, null, "errored relay-self state must block any DM");
});

test("checkSendEligibility_relay_self_null_success_ordinary_dm_eligible", () => {
  // Semantic boundary: relaySelf successfully resolved to null means the relay
  // advertises no self pubkey (a known, valid answer).  isModerationDm returns
  // false when relaySelf is null/undefined.  The DM must be eligible.
  const MY_PUBKEY = "mypubkey".padEnd(64, "0");
  const OTHER_PUBKEY = "other000".padEnd(64, "0");
  const qc = makeMockQueryClient({
    '["channels"]': [
      makeChannel({
        id: "ch-dm-null-relay",
        channelType: "dm",
        participantPubkeys: [MY_PUBKEY, OTHER_PUBKEY],
      }),
    ],
    '["identity"]': { pubkey: MY_PUBKEY, displayName: "Me" },
    '["relaySelf"]': null, // successfully resolved: relay has no self
    'state:["identity"]': { status: "success", fetchStatus: "idle" },
    'state:["relaySelf"]': { status: "success", fetchStatus: "idle" },
  });
  const result = checkSendEligibility(qc, "ch-dm-null-relay", 1000);
  assert.equal(
    result,
    null,
    "relaySelf=null (known: relay has no self) + ordinary DM must be eligible",
  );
});
