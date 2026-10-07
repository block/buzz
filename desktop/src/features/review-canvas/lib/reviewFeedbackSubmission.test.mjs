import assert from "node:assert/strict";
import { test } from "node:test";

import { RelayEventRejectedError } from "@/shared/api/relayEventRejection.ts";
import { parseReviewRevision } from "./reviewContract.ts";
import { ReviewFeedbackSubmission } from "./reviewFeedbackSubmission.ts";
import {
  createReviewOutbox,
  identityOf,
  OUTBOX_ABANDONABLE_TTL_SECONDS,
  OUTBOX_MAX_ENTRIES,
} from "./reviewOutbox.ts";
import {
  AGENT,
  FEEDBACK_ID,
  memoryStorage,
  NOTIFICATION_ID,
  ORIGIN_EVENT,
  REVIEWER,
  REVISION_EVENT,
  reviewEvent,
} from "./reviewFixtures.mjs";

const BLOCK = {
  id: "checkout.primary-action",
  title: "Primary checkout action",
  sourceRef: "src/features/checkout/CheckoutActions.tsx",
};
const RELAY = "wss://relay.example";

/** Deterministic fakes recording every call the submission makes. */
function harness(overrides = {}) {
  const state = {
    relay: RELAY,
    signer: REVIEWER,
    clock: 1_700_000_000,
    /** Event IDs the fake relay stores, as `reconcileFeedback` reports. */
    held: new Set(),
    /** When set, the relay cannot be read for an exact event. */
    reconcileError: null,
  };
  const storage = memoryStorage();
  const outbox = createReviewOutbox(storage, () => state.clock);
  const log = {
    signed: [],
    published: [],
    notified: [],
    reconciled: [],
    headChecks: 0,
    guards: [],
  };
  let counter = 0;
  const deps = {
    newFeedbackId: () => FEEDBACK_ID,
    nowSeconds: () => {
      state.clock += 1;
      return state.clock;
    },
    signEvent: async (template) => {
      counter += 1;
      const event = {
        id: String(counter).repeat(64).slice(0, 64),
        pubkey: REVIEWER,
        created_at: 1_700_000_000 + counter,
        kind: template.kind,
        tags: template.tags,
        content: template.content,
        sig: "f".repeat(128),
      };
      log.signed.push(event);
      return event;
    },
    publishEvent: async (event, isCurrent) => {
      log.guards.push(isCurrent);
      log.published.push(event);
    },
    sendNotification: async (input) => {
      log.notified.push(input);
    },
    isScopeCurrent: (context) =>
      state.relay === context.expectedRelayUrl &&
      state.signer === context.expectedSignerPubkey,
    assertHeadCurrent: async () => {
      log.headChecks += 1;
    },
    // The relay holds nothing unless a test says otherwise.
    reconcileFeedback: async (input) => {
      log.reconciled.push(input);
      if (state.reconcileError) throw new Error(state.reconcileError);
      return state.held.has(input.event.id) ? "held" : "absent";
    },
    outbox,
    ...overrides,
  };
  const context = {
    revision: parseReviewRevision(reviewEvent()),
    block: BLOCK,
    request: "Move this above the order summary.",
    viewport: { width: 1440, height: 900 },
    executiveAgentPubkey: AGENT,
    parentEventId: NOTIFICATION_ID,
    rootEventId: ORIGIN_EVENT,
    expectedRelayUrl: RELAY,
    expectedSignerPubkey: REVIEWER,
  };
  return { log, deps, context, state, storage, outbox };
}

test("one submit signs and publishes one feedback revision, then one wake message", async () => {
  const { log, deps, context } = harness();
  const submission = new ReviewFeedbackSubmission(context, deps);
  const receipt = await submission.submit();

  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 1);
  assert.equal(log.notified.length, 1);
  assert.equal(log.published[0].id, log.signed[0].id);
  assert.deepEqual(receipt, {
    feedbackEventId: log.signed[0].id,
    feedbackArtifactId: FEEDBACK_ID,
  });

  const wake = log.notified[0];
  assert.equal(wake.feedbackRevisionEventId, log.signed[0].id);
  assert.equal(wake.feedbackArtifactId, FEEDBACK_ID);
  assert.equal(wake.reviewedRevisionEventId, REVISION_EVENT);
  assert.equal(wake.executiveAgentPubkey, AGENT);
  assert.equal(wake.parentEventId, NOTIFICATION_ID);
  assert.equal(wake.rootEventId, ORIGIN_EVENT);
  assert.equal(wake.channelId, context.revision.channelId);
  assert.equal(wake.expectedRelayUrl, RELAY);
  assert.equal(wake.expectedSignerPubkey, REVIEWER);
  assert.equal(wake.createdAt, 1_700_000_001);
  assert.match(wake.content, /Primary checkout action/);
});

test("a failed feedback publish never sends the wake message", async () => {
  const { log, deps, context } = harness({
    publishEvent: async () => {
      throw new Error("relay rejected the feedback revision");
    },
  });
  const submission = new ReviewFeedbackSubmission(context, deps);
  await assert.rejects(submission.submit(), /relay rejected/);
  assert.equal(log.notified.length, 0);
  assert.equal(submission.feedbackPublished, false);
  assert.equal(submission.completed, false);
});

test("retry republishes the identical frozen signed event, never re-signing", async () => {
  let attempts = 0;
  const { log, deps, context } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    publishEvent: async (event) => {
      attempts += 1;
      log.published.push(event);
      if (attempts === 1) throw new Error("timed out");
    },
  });

  await assert.rejects(submission.submit(), /timed out/);
  assert.equal(submission.frozen, true);
  await submission.submit();

  assert.equal(log.signed.length, 1, "the first signed payload is reused");
  assert.equal(log.published.length, 2);
  assert.deepEqual(log.published[0], log.published[1]);
  assert.equal(log.notified.length, 1);
});

test("accept-then-lost-ack: the retried wake carries the identical frozen inputs, so it is one event", async () => {
  let attempts = 0;
  const { log, deps, context, state } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    sendNotification: async (input) => {
      attempts += 1;
      log.notified.push(input);
      // The relay accepted the first wake, but the acknowledgement never arrived.
      if (attempts === 1)
        throw new Error("relay unreachable: request timed out");
    },
  });

  await assert.rejects(submission.submit(), /timed out/);
  assert.equal(submission.feedbackPublished, true);
  // Time passes before the reviewer retries: a rebuilt wake would get a new
  // timestamp and therefore a new event ID.
  state.clock += 600;
  await submission.submit();

  assert.equal(log.published.length, 1, "feedback is not republished");
  assert.equal(log.notified.length, 2);
  // Every field that feeds the wake event ID is identical across attempts.
  assert.deepEqual(log.notified[0], log.notified[1]);
  assert.equal(log.notified[1].createdAt, 1_700_000_001);
  assert.equal(submission.completed, true);
});

test("concurrent submits share one in-flight run (duplicate-submit guard)", async () => {
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const { log, deps, context } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    publishEvent: async (event) => {
      await gate;
      log.published.push(event);
    },
  });
  const first = submission.submit();
  const second = submission.submit();
  const third = submission.submit();
  release();
  await Promise.all([first, second, third]);

  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 1);
  assert.equal(log.notified.length, 1);
});

test("submitting a completed submission again sends nothing new", async () => {
  const { log, deps, context } = harness();
  const submission = new ReviewFeedbackSubmission(context, deps);
  await submission.submit();
  await submission.submit();
  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 1);
  assert.equal(log.notified.length, 1);
});

test("a signer that alters the composed event is refused before publishing", async () => {
  for (const tamper of [
    (event) => ({ ...event, content: `${event.content} ` }),
    (event) => ({ ...event, tags: [...event.tags, ["extra", "x"]] }),
    (event) => ({ ...event, kind: 9 }),
    (event) => ({ ...event, pubkey: "9".repeat(64) }),
    (event) => ({ ...event, id: "not-an-id" }),
  ]) {
    const { log, deps, context } = harness();
    const submission = new ReviewFeedbackSubmission(context, {
      ...deps,
      signEvent: async (template) => tamper(await deps.signEvent(template)),
    });
    await assert.rejects(submission.submit(), /did not match/);
    assert.equal(log.published.length, 0);
    assert.equal(log.notified.length, 0);
    assert.equal(submission.frozen, false);
  }
});

test("blank comments, unidentifiable agents, and a missing captured scope never reach the signer", () => {
  const { log, deps, context } = harness();
  for (const bad of [
    { request: "  " },
    { executiveAgentPubkey: "agent" },
    { expectedRelayUrl: "" },
    { expectedSignerPubkey: "" },
    { expectedSignerPubkey: "reviewer" },
  ]) {
    assert.throws(
      () => new ReviewFeedbackSubmission({ ...context, ...bad }, deps),
      undefined,
      JSON.stringify(bad),
    );
  }
  assert.equal(log.signed.length, 0);
});

// ── Captured tenant scope ───────────────────────────────────────────────────

test("a community switch during signing stops the publish: nothing reaches the new relay", async () => {
  const { log, deps, context, state } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    signEvent: async (template) => {
      const event = await deps.signEvent(template);
      state.relay = "wss://other-tenant.example";
      return event;
    },
  });
  await assert.rejects(submission.submit(), /community or account changed/);
  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 0, "the signed comment was never sent");
  assert.equal(log.notified.length, 0);

  // A retry in the wrong community stays blocked; back in the original one it
  // republishes the same frozen event.
  await assert.rejects(submission.submit(), /community or account changed/);
  assert.equal(log.published.length, 0);
  state.relay = RELAY;
  await submission.submit();
  assert.equal(log.signed.length, 1);
  assert.deepEqual(
    log.published.map((event) => event.id),
    [log.signed[0].id],
  );
});

test("an account switch is refused the same way", async () => {
  const { log, deps, context, state } = harness();
  state.signer = "8".repeat(64);
  await assert.rejects(
    new ReviewFeedbackSubmission(context, deps).submit(),
    /community or account changed/,
  );
  assert.equal(log.signed.length, 0);
});

test("publishing hands the relay client a live scope guard for every send and retry", async () => {
  const { log, deps, context, state } = harness();
  await new ReviewFeedbackSubmission(context, deps).submit();
  assert.equal(log.guards.length, 1);
  const isCurrent = log.guards[0];
  assert.equal(isCurrent(), true);
  state.relay = "wss://other-tenant.example";
  assert.equal(
    isCurrent(),
    false,
    "the guard reads live state, not a snapshot",
  );
  state.relay = RELAY;
  state.signer = "8".repeat(64);
  assert.equal(isCurrent(), false);
});

test("the wake is never attempted once the scope changed after publishing", async () => {
  const { log, deps, context, state } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    publishEvent: async (event) => {
      log.published.push(event);
      state.relay = "wss://other-tenant.example";
    },
  });
  await assert.rejects(submission.submit(), /community or account changed/);
  assert.equal(submission.feedbackPublished, true);
  assert.equal(log.notified.length, 0);
});

// ── Head checks gate new events, never a frozen one ─────────────────────────

const STALE_HEAD = "A newer revision of this review exists";

test("the head is checked once before a new event is signed, and not again before publishing", async () => {
  const order = [];
  const { log, deps, context } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    assertHeadCurrent: async () => {
      order.push("head");
    },
    signEvent: async (template) => {
      order.push("sign");
      return deps.signEvent(template);
    },
    publishEvent: async (event) => {
      order.push("publish");
      log.published.push(event);
    },
  });
  await submission.submit();
  assert.deepEqual(order, ["head", "sign", "publish"]);
});

test("a stale or unverifiable head stops everything: fail closed", async () => {
  const { log, deps, context } = harness({
    assertHeadCurrent: async () => {
      throw new Error(STALE_HEAD);
    },
  });
  const submission = new ReviewFeedbackSubmission(context, deps);
  await assert.rejects(submission.submit(), /newer revision/);
  assert.equal(log.signed.length, 0, "nothing was even signed");
  assert.equal(log.published.length, 0);
  assert.equal(log.notified.length, 0);
  assert.equal(submission.frozen, false, "an unsigned draft is not frozen");
});

test("an unsigned draft is head-checked again on every attempt until it signs", async () => {
  let healthy = false;
  const { log, deps, context } = harness({
    assertHeadCurrent: async () => {
      log.headChecks += 1;
      if (!healthy) throw new Error("Could not read the review head");
    },
  });
  const submission = new ReviewFeedbackSubmission(context, deps);
  await assert.rejects(submission.submit(), /Could not read/);
  await assert.rejects(submission.submit(), /Could not read/);
  assert.equal(log.signed.length, 0);
  healthy = true;
  await submission.submit();
  assert.equal(log.headChecks, 3);
  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 1);
  assert.equal(log.notified.length, 1);
});

/**
 * A relay that accepts the first EVENT but loses its acknowledgement, then
 * answers a republish like the real one: success for an event it already
 * holds, a stale conflict otherwise.
 */
function lostAckRelay(log, { stored }) {
  const accepted = new Set();
  let attempts = 0;
  return async (event) => {
    attempts += 1;
    log.published.push(event);
    if (attempts === 1 && stored) accepted.add(event.id);
    if (attempts === 1) throw new Error("relay unreachable: request timed out");
    if (!accepted.has(event.id)) {
      throw new Error(
        "conflict: feedback target is not the current review revision",
      );
    }
  };
}

for (const [label, headFailure] of [
  ["advanced", STALE_HEAD],
  ["unreadable", "Could not read the review head"],
]) {
  test(`a lost feedback acknowledgement republishes the identical event with the head ${label}, without a head check`, async () => {
    const { log, deps, context } = harness();
    let headHealthy = true;
    const submission = new ReviewFeedbackSubmission(context, {
      ...deps,
      assertHeadCurrent: async () => {
        log.headChecks += 1;
        if (!headHealthy) throw new Error(headFailure);
      },
      publishEvent: lostAckRelay(log, { stored: true }),
    });

    await assert.rejects(submission.submit(), /timed out/);
    assert.equal(submission.frozen, true);
    assert.equal(submission.feedbackPublished, false);
    // The relay holds the feedback, but the head moved (or cannot be read)
    // before the reviewer pressed Retry.
    headHealthy = false;
    await submission.submit();

    assert.equal(log.headChecks, 1, "only the check before signing happened");
    assert.equal(log.signed.length, 1, "no second feedback revision is signed");
    assert.equal(log.published.length, 2);
    assert.deepEqual(log.published[0], log.published[1]);
    assert.equal(log.notified.length, 1, "exactly one wake");
    assert.equal(submission.completed, true);
  });
}

test("a frozen event the relay never accepted stays frozen on a stale conflict and sends no wake", async () => {
  const { log, deps, context } = harness();
  const submission = new ReviewFeedbackSubmission(context, {
    ...deps,
    publishEvent: lostAckRelay(log, { stored: false }),
  });
  await assert.rejects(submission.submit(), /timed out/);
  await assert.rejects(submission.submit(), /conflict: feedback target/);

  assert.equal(submission.frozen, true);
  assert.equal(submission.feedbackPublished, false);
  assert.equal(log.signed.length, 1);
  assert.deepEqual(log.published[0], log.published[1]);
  assert.equal(log.notified.length, 0, "no wake for refused feedback");
});

for (const [label, headFailure] of [
  ["advanced", STALE_HEAD],
  ["unreadable", "Could not read the review head"],
]) {
  test(`accepted feedback retries its identical wake with the head ${label}, without a head check`, async () => {
    const { log, deps, context, state } = harness();
    let headHealthy = true;
    let wakeAttempts = 0;
    const submission = new ReviewFeedbackSubmission(context, {
      ...deps,
      assertHeadCurrent: async () => {
        log.headChecks += 1;
        if (!headHealthy) throw new Error(headFailure);
      },
      sendNotification: async (input) => {
        wakeAttempts += 1;
        log.notified.push(input);
        if (wakeAttempts === 1) throw new Error("wake submission failed");
      },
    });

    await assert.rejects(submission.submit(), /wake submission failed/);
    assert.equal(submission.feedbackPublished, true);
    headHealthy = false;
    state.clock += 600;
    await submission.submit();

    assert.equal(log.headChecks, 1, "only the check before signing happened");
    assert.equal(log.signed.length, 1);
    assert.equal(log.published.length, 1, "feedback is not republished");
    assert.equal(log.notified.length, 2);
    assert.deepEqual(log.notified[0], log.notified[1]);
    assert.equal(submission.completed, true);
  });
}

// ── Publish outcomes are explicit: ambiguous is never discardable ───────────

const TIMED_OUT = () => new Error("relay unreachable: request timed out");
const REFUSED = (
  reason = "conflict: feedback target is not the current review revision",
) => new RelayEventRejectedError(reason);
const entryOf = (h) => h.outbox.load(identityOf(h.context));

test("the signed event is on disk before it is sent, and the attempt is recorded before the send starts", async () => {
  const h = harness();
  const seenDuringSend = [];
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async (event) => {
      // Observed from inside the send: the write-ahead state, already durable.
      seenDuringSend.push(
        entryOf(h)?.publish,
        entryOf(h)?.signed.id === event.id,
      );
      h.log.published.push(event);
    },
  });
  await submission.submit();
  assert.deepEqual(seenDuringSend, ["ambiguous", true]);
  assert.equal(
    entryOf(h),
    undefined,
    "cleared once feedback and wake finished",
  );
  assert.equal(h.storage.data.size, 0);
});

test("a publish that ends without the relay's answer is ambiguous and can never be discarded", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async () => {
      throw TIMED_OUT();
    },
  });
  await assert.rejects(submission.submit(), /timed out/);
  assert.equal(submission.publishState, "ambiguous");
  assert.equal(submission.canDiscard, false);
  await assert.rejects(submission.discard(), /may already be on the relay/);
  assert.equal(entryOf(h)?.publish, "ambiguous");
  assert.equal(
    h.log.reconciled.length,
    0,
    "refused without even asking the relay",
  );
});

test("only an explicit relay refusal makes a publish rejected", async () => {
  for (const [error, expected] of [
    [REFUSED(), "rejected"],
    [REFUSED("invalid: event timestamp too far from server time"), "rejected"],
    [REFUSED("restricted: not a member"), "rejected"],
    // A backend fault can strike after the commit, and a transport error says
    // nothing; neither is a refusal.
    [REFUSED("error: internal server error"), "ambiguous"],
    [REFUSED("relay unreachable: request timed out"), "ambiguous"],
    [new Error("conflict: looks like a refusal but is untyped"), "ambiguous"],
  ]) {
    const h = harness();
    const submission = new ReviewFeedbackSubmission(h.context, {
      ...h.deps,
      publishEvent: async () => {
        throw error;
      },
    });
    await assert.rejects(submission.submit());
    assert.equal(submission.publishState, expected, error.message);
    assert.equal(entryOf(h)?.publish, expected, error.message);
    assert.equal(submission.canDiscard, expected === "rejected");
  }
});

test("lost acknowledgement: the relay holds the event, so Discard is refused and Retry adopts it without resending", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async (event) => {
      h.log.published.push(event);
      h.state.held.add(event.id); // committed; only the acknowledgement is lost
      throw TIMED_OUT();
    },
  });
  await assert.rejects(submission.submit(), /timed out/);
  assert.equal(submission.canDiscard, false);
  await assert.rejects(submission.discard(), /may already be on the relay/);
  assert.ok(entryOf(h), "the only copy of the signed event is kept");

  await submission.submit();
  assert.equal(h.log.signed.length, 1, "no replacement is signed");
  assert.equal(
    h.log.published.length,
    1,
    "the relay already held it: not resent",
  );
  assert.equal(h.log.reconciled.at(-1).event.id, h.log.signed[0].id);
  assert.equal(h.log.notified.length, 1);
  assert.equal(submission.completed, true);
  assert.equal(entryOf(h), undefined);
});

test("a refused comment is discardable only after a fresh read confirms the relay holds nothing", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async () => {
      throw REFUSED();
    },
  });
  await assert.rejects(submission.submit(), /conflict/);
  assert.equal(submission.publishState, "rejected");
  assert.equal(submission.canDiscard, true);

  // The relay cannot be read: nothing proves it holds nothing, so it is kept.
  h.state.reconcileError = "relay unreachable: request timed out";
  await assert.rejects(submission.discard(), /could not confirm/i);
  assert.equal(submission.frozen, true);
  assert.ok(entryOf(h));

  // The refusal raced a commit and the relay holds it after all: kept, accepted.
  h.state.reconcileError = null;
  h.state.held.add(submission.feedbackEventId);
  assert.equal(await submission.discard(), "held");
  assert.equal(submission.publishState, "accepted");
  assert.equal(submission.canDiscard, false);
  await assert.rejects(submission.discard(), /cannot be discarded/);
  assert.equal(entryOf(h)?.publish, "accepted");
});

test("a refused comment the relay confirms absent is abandoned for good", async () => {
  const h = harness();
  let sends = 0;
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async () => {
      sends += 1;
      throw REFUSED();
    },
  });
  await assert.rejects(submission.submit(), /conflict/);
  assert.equal(await submission.discard(), "discarded");
  assert.equal(submission.frozen, false);
  assert.equal(entryOf(h), undefined);
  assert.equal(h.storage.data.size, 0);
  await assert.rejects(submission.submit(), /discarded/);
  assert.equal(sends, 1, "nothing more was ever sent");
  assert.equal(h.log.notified.length, 0);
});

test("a comment that was signed but never sent can be discarded without asking the relay", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    signEvent: async (template) => {
      const event = await h.deps.signEvent(template);
      h.state.relay = "wss://other-tenant.example";
      return event;
    },
  });
  await assert.rejects(submission.submit(), /community or account changed/);
  assert.equal(submission.publishState, "never_attempted");
  assert.equal(entryOf(h)?.publish, "never_attempted");
  assert.equal(submission.canDiscard, true);
  assert.equal(await submission.discard(), "discarded");
  assert.equal(h.log.reconciled.length, 0);
  assert.equal(entryOf(h), undefined);
  assert.equal(h.log.published.length, 0);
});

test("reconcile adopts a comment the relay holds, leaves the rest alone, and never throws", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async () => {
      throw TIMED_OUT();
    },
  });
  await assert.rejects(submission.submit());

  h.state.reconcileError = "relay unreachable: request timed out";
  await submission.reconcile();
  assert.equal(submission.publishState, "ambiguous", "unreachable: unchanged");

  h.state.reconcileError = null;
  await submission.reconcile();
  assert.equal(submission.publishState, "ambiguous", "absent: unchanged");

  h.state.held.add(submission.feedbackEventId);
  h.state.relay = "wss://other-tenant.example";
  await submission.reconcile();
  assert.equal(submission.publishState, "ambiguous", "wrong tenant: not read");

  h.state.relay = RELAY;
  await submission.reconcile();
  assert.equal(submission.publishState, "accepted");
  assert.equal(entryOf(h)?.publish, "accepted");
});

// ── The durable outbox: restart, bounds, and fail-closed persistence ────────

test("a restart resumes the exact comment from disk: same event, same state, same frozen wake", async () => {
  const h = harness();
  const first = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    sendNotification: async (input) => {
      h.log.notified.push(input);
      throw new Error("wake submission failed");
    },
  });
  await assert.rejects(first.submit(), /wake submission failed/);
  assert.equal(first.publishState, "accepted");

  // Quit and relaunch: nothing survives except what is on disk. A fresh outbox
  // over the same storage has no parsed cache and no live submission.
  const relaunched = createReviewOutbox(h.storage, () => h.state.clock);
  const entry = relaunched.load(identityOf(h.context));
  assert.equal(entry.publish, "accepted");
  assert.equal(entry.signed.id, h.log.signed[0].id);
  assert.deepEqual(entry.wake, h.log.notified[0]);

  const restored = ReviewFeedbackSubmission.restore(entry, {
    ...h.deps,
    outbox: relaunched,
  });
  assert.equal(restored.feedbackEventId, h.log.signed[0].id);
  assert.equal(restored.feedbackPublished, true);
  assert.equal(restored.request, first.request);
  h.state.clock += 2_000;
  await restored.submit();

  assert.equal(h.log.signed.length, 1, "no second feedback revision");
  assert.equal(h.log.published.length, 1, "feedback is not republished");
  assert.equal(h.log.notified.length, 2);
  assert.deepEqual(
    h.log.notified[0],
    h.log.notified[1],
    "the wake's frozen inputs, timestamp included, survive the restart",
  );
  assert.equal(restored.completed, true);
  assert.equal(relaunched.load(identityOf(h.context)), undefined);
  assert.equal(h.storage.data.size, 0, "cleared only after both finished");
});

test("an ambiguous comment restarts as ambiguous, still undiscardable, and is adopted if the relay holds it", async () => {
  const h = harness();
  const first = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async (event) => {
      h.log.published.push(event);
      h.state.held.add(event.id);
      throw TIMED_OUT();
    },
  });
  await assert.rejects(first.submit(), /timed out/);

  const relaunched = createReviewOutbox(h.storage, () => h.state.clock);
  const restored = ReviewFeedbackSubmission.restore(
    relaunched.load(identityOf(h.context)),
    { ...h.deps, outbox: relaunched },
  );
  assert.equal(restored.publishState, "ambiguous");
  assert.equal(restored.canDiscard, false);
  assert.equal(restored.feedbackEventId, h.log.signed[0].id);
  await restored.submit();
  assert.equal(h.log.published.length, 1);
  assert.equal(h.log.notified.length, 1);
  assert.equal(h.log.notified[0].feedbackRevisionEventId, h.log.signed[0].id);
});

test("a process that dies mid-send restarts as ambiguous, never as never-sent", async () => {
  const h = harness();
  let died;
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: () =>
      new Promise((_, reject) => {
        died = reject; // the send never answers
      }),
  });
  const run = submission.submit().catch(() => undefined);
  await new Promise((resolve) => setImmediate(resolve));
  // What is on disk at this instant is what a crash here would leave.
  const relaunched = createReviewOutbox(h.storage, () => h.state.clock);
  const onDisk = relaunched.load(identityOf(h.context));
  assert.equal(onDisk.publish, "ambiguous");
  const restored = ReviewFeedbackSubmission.restore(onDisk, {
    ...h.deps,
    outbox: relaunched,
  });
  assert.equal(restored.canDiscard, false);
  died(TIMED_OUT());
  await run;
});

test("nothing is sent when the signed comment cannot be saved", async () => {
  const h = harness();
  h.storage.broken = true;
  const submission = new ReviewFeedbackSubmission(h.context, h.deps);
  await assert.rejects(submission.submit(), /could not be saved/);
  assert.equal(h.log.published.length, 0);
  assert.equal(h.log.notified.length, 0);
  assert.equal(submission.frozen, false, "an unsaved signature is not kept");
});

test("nothing is sent when the attempt cannot be recorded ahead of the send", async () => {
  const h = harness();
  const write = h.storage.setItem;
  let writes = 0;
  h.storage.setItem = (key, value) => {
    writes += 1;
    // The first write saves the signed event; the second is the write-ahead.
    if (writes === 2) throw new Error("QuotaExceededError");
    write(key, value);
  };
  const submission = new ReviewFeedbackSubmission(h.context, h.deps);
  await assert.rejects(submission.submit(), /could not be saved/);
  assert.equal(h.log.published.length, 0);
  assert.equal(submission.publishState, "never_attempted");
  assert.equal(entryOf(h)?.publish, "never_attempted");
});

test("a failed best-effort save never blocks finishing: the stored state only lags safely", async () => {
  const h = harness();
  const write = h.storage.setItem;
  let writes = 0;
  h.storage.setItem = (key, value) => {
    writes += 1;
    // Saves 1 and 2 (signed, write-ahead) succeed; the `accepted` save fails.
    if (writes === 3) throw new Error("QuotaExceededError");
    write(key, value);
  };
  const submission = new ReviewFeedbackSubmission(h.context, h.deps);
  await submission.submit();
  assert.equal(submission.completed, true);
  assert.equal(h.log.notified.length, 1);
});

test("the outbox is bounded: once full, a new comment is refused before anything is signed", async () => {
  const h = harness();
  for (let index = 0; index < OUTBOX_MAX_ENTRIES; index += 1) {
    const submission = new ReviewFeedbackSubmission(
      {
        ...h.context,
        block: {
          id: `block.${index}`,
          title: `Block ${index}`,
          sourceRef: null,
        },
      },
      {
        ...h.deps,
        newFeedbackId: () => crypto.randomUUID(),
        publishEvent: async () => {
          throw TIMED_OUT();
        },
      },
    );
    await assert.rejects(submission.submit(), /timed out/);
  }
  assert.equal(h.log.signed.length, OUTBOX_MAX_ENTRIES);
  const extra = new ReviewFeedbackSubmission(
    {
      ...h.context,
      block: { id: "block.extra", title: "Extra", sourceRef: null },
    },
    { ...h.deps, newFeedbackId: () => crypto.randomUUID() },
  );
  await assert.rejects(extra.submit(), /at most 16 unfinished comments/);
  assert.equal(h.log.signed.length, OUTBOX_MAX_ENTRIES, "never signed");
  assert.equal(extra.frozen, false);
});

test("a different comment cannot take the place of one the relay may hold", async () => {
  const h = harness();
  const first = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async () => {
      throw TIMED_OUT();
    },
  });
  await assert.rejects(first.submit(), /timed out/);

  const replacement = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    newFeedbackId: () => "34737c81-e5e8-4412-bb47-f446813cfeba",
  });
  await assert.rejects(replacement.submit(), /already has a comment waiting/);
  assert.equal(h.log.signed.length, 1, "a replacement is not even signed");
  assert.equal(entryOf(h).signed.id, h.log.signed[0].id);
  assert.equal(h.log.published.length, 0);
});

test("stored records are validated: a record that no longer matches its signed event is dropped", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    publishEvent: async () => {
      throw TIMED_OUT();
    },
  });
  await assert.rejects(submission.submit());
  const [key, raw] = [...h.storage.data.entries()][0];

  const tamper = (edit) => {
    const blob = JSON.parse(raw);
    edit(blob.entries[0]);
    return createReviewOutbox(
      memoryStorage({ [key]: JSON.stringify(blob) }),
      () => h.state.clock,
    ).load(identityOf(h.context));
  };
  assert.ok(
    tamper(() => {}),
    "an untouched record loads",
  );
  assert.equal(
    tamper((entry) => (entry.context.request = "Something else")),
    undefined,
  );
  assert.equal(
    tamper(
      (entry) =>
        (entry.context.revision.synaxisArtifact.payloadDigest = "9".repeat(64)),
    ),
    undefined,
  );
  assert.equal(
    tamper((entry) => (entry.signed.content += " ")),
    undefined,
  );
  assert.equal(
    tamper((entry) => (entry.signed.pubkey = "9".repeat(64))),
    undefined,
  );
  assert.equal(
    tamper((entry) => (entry.publish = "nonsense")),
    undefined,
  );
  // A wake that disagrees with its comment is dropped, not trusted; the
  // comment itself survives and rebuilds it.
  const wake = {
    channelId: h.context.revision.channelId,
    content: "Feedback on a block",
    parentEventId: NOTIFICATION_ID,
    rootEventId: ORIGIN_EVENT,
    executiveAgentPubkey: AGENT,
    feedbackArtifactId: FEEDBACK_ID,
    feedbackRevisionEventId: h.log.signed[0].id,
    reviewedArtifactId: h.context.revision.artifactId,
    reviewedRevisionEventId: h.context.revision.eventId,
    createdAt: 1_700_000_001,
    expectedRelayUrl: RELAY,
    expectedSignerPubkey: REVIEWER,
  };
  assert.deepEqual(tamper((entry) => (entry.wake = wake))?.wake, wake);
  for (const wrong of [
    { feedbackRevisionEventId: "9".repeat(64) },
    { reviewedRevisionEventId: "9".repeat(64) },
    { executiveAgentPubkey: "9".repeat(64) },
    { expectedRelayUrl: "wss://other-tenant.example" },
    { createdAt: -1 },
  ]) {
    assert.equal(
      tamper((entry) => (entry.wake = { ...wake, ...wrong }))?.wake,
      null,
      JSON.stringify(wrong),
    );
  }
});

test("only a record the relay provably holds nothing for ever expires", async () => {
  const h = harness();
  const seed = async (block, error) => {
    const submission = new ReviewFeedbackSubmission(
      { ...h.context, block },
      {
        ...h.deps,
        newFeedbackId: () => crypto.randomUUID(),
        publishEvent: async () => {
          throw error;
        },
      },
    );
    await assert.rejects(submission.submit());
    return submission;
  };
  const refusedBlock = {
    id: "block.refused",
    title: "Refused",
    sourceRef: null,
  };
  const unknownBlock = {
    id: "block.unknown",
    title: "Unknown",
    sourceRef: null,
  };
  const refused = await seed(refusedBlock, REFUSED());
  const unknown = await seed(unknownBlock, TIMED_OUT());
  assert.equal(refused.publishState, "rejected");
  assert.equal(unknown.publishState, "ambiguous");

  // Well past the retention window, on a freshly launched app.
  h.state.clock += OUTBOX_ABANDONABLE_TTL_SECONDS + 1;
  const relaunched = createReviewOutbox(h.storage, () => h.state.clock);
  const load = (block) => relaunched.load(identityOf({ ...h.context, block }));
  assert.equal(load(refusedBlock), undefined, "refused for 30 days: expired");
  assert.equal(
    load(unknownBlock)?.publish,
    "ambiguous",
    "the relay may hold it, and its wake may be owed: never expired",
  );
});

test("the outbox holds only public data: no key material is ever written", async () => {
  const h = harness();
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    sendNotification: async () => {
      throw new Error("wake submission failed");
    },
  });
  await assert.rejects(submission.submit());
  const [, raw] = [...h.storage.data.entries()][0];
  const [entry] = JSON.parse(raw).entries;
  assert.deepEqual(Object.keys(entry).sort(), [
    "context",
    "createdAt",
    "feedbackArtifactId",
    "publish",
    "signed",
    "updatedAt",
    "wake",
  ]);
  assert.deepEqual(Object.keys(entry.signed).sort(), [
    "content",
    "created_at",
    "id",
    "kind",
    "pubkey",
    "sig",
    "tags",
  ]);
  assert.equal(/nsec|secret|private|mnemonic/i.test(raw), false);
});

// ── The wake: retry with a fresh timestamp rests on the relay's ledger ──────

test("the frozen wake inputs are saved at the first wake attempt and never change", async () => {
  const h = harness();
  let attempts = 0;
  const submission = new ReviewFeedbackSubmission(h.context, {
    ...h.deps,
    sendNotification: async (input) => {
      attempts += 1;
      h.log.notified.push(input);
      if (attempts < 3) throw new Error("relay unreachable: request timed out");
    },
  });
  await assert.rejects(submission.submit());
  const saved = entryOf(h).wake;
  assert.deepEqual(saved, h.log.notified[0]);
  // More than the relay's 900 s freshness window passes between retries.
  h.state.clock += 1_000;
  await assert.rejects(submission.submit());
  h.state.clock += 1_000;
  await submission.submit();
  assert.equal(h.log.notified.length, 3);
  assert.deepEqual(h.log.notified[1], saved);
  assert.deepEqual(
    h.log.notified[2],
    saved,
    "the renderer never re-stamps the wake; the native layer refreshes it only on a stale refusal",
  );
});
