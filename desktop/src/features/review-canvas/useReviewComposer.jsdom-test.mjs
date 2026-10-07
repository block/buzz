/**
 * The Review Canvas composer, rendered through real React mounts.
 *
 * Contract: a signed comment survives its workbench unmounting, returns locked
 * and retryable for exactly the relay, signer, and reviewed revision it was
 * signed for, never carries into another revision, stays fail-closed when the
 * captured community or account is no longer active, and is forgotten only
 * once feedback and wake both completed or the reviewer discards one the relay
 * never accepted.
 */
import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { RelayEventRejectedError } from "@/shared/api/relayEventRejection.ts";
import { parseReviewRevision } from "./lib/reviewContract.ts";
import {
  AGENT,
  FEEDBACK_ID,
  memoryStorage,
  NOTIFICATION_ID,
  ORIGIN_EVENT,
  REVIEWER,
  REVISION_TWO_EVENT,
  reviewEvent,
} from "./lib/reviewFixtures.mjs";
import { ReviewFeedbackSubmission } from "./lib/reviewFeedbackSubmission.ts";
import { createReviewOutbox, OUTBOX_MAX_ENTRIES } from "./lib/reviewOutbox.ts";
import { resetSubmissionStore } from "./lib/reviewSubmissionStore.ts";
import { liveFeedbackDeps, useReviewComposer } from "./useReviewComposer.ts";

const RELAY = "wss://relay.example";
const OTHER_RELAY = "wss://other.example";
const PRIMARY = "checkout.primary-action";
const BLOCKS = [
  { id: "checkout.header", title: "Checkout header", sourceRef: null },
  { id: PRIMARY, title: "Primary checkout action", sourceRef: null },
];
const COMMENT = "Move this above the order summary.";

const revisionOne = parseReviewRevision(reviewEvent());
const revisionTwo = parseReviewRevision(
  reviewEvent({ eventId: REVISION_TWO_EVENT, revision: 2 }),
);

/** Live, mutable world the fake dependencies read, like the real scope does. */
let scope;
let failure;
let log;
let latest;
let deps;
/** The disk behind the durable outbox, and the event IDs the relay stores. */
let storage;
let held;

beforeEach(() => {
  resetSubmissionStore();
  scope = { relayUrl: RELAY, signerPubkey: REVIEWER };
  failure = {
    publish: null,
    // The relay stores the event but its acknowledgement is lost.
    publishStored: false,
    // An explicit relay refusal (a typed rejection), as opposed to `publish`.
    refusal: null,
    wake: null,
    // The relay cannot be read for an exact event.
    reconcile: null,
  };
  log = {
    signed: [],
    published: [],
    publishCalls: 0,
    notified: [],
    reconciled: [],
    sent: 0,
  };
  storage = memoryStorage();
  held = new Set();
  let counter = 0;
  deps = {
    newFeedbackId: () => FEEDBACK_ID,
    nowSeconds: () => 1_700_000_500,
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
    publishEvent: async (event) => {
      log.publishCalls += 1;
      if (failure.refusal) throw new RelayEventRejectedError(failure.refusal);
      if (failure.publish) {
        if (failure.publishStored) held.add(event.id);
        throw new Error(failure.publish);
      }
      held.add(event.id);
      log.published.push(event);
    },
    sendNotification: async (input) => {
      if (failure.wake) throw new Error(failure.wake);
      log.notified.push(input);
    },
    isScopeCurrent: (context) =>
      scope.relayUrl === context.expectedRelayUrl &&
      scope.signerPubkey === context.expectedSignerPubkey,
    assertHeadCurrent: async () => {},
    reconcileFeedback: async (input) => {
      log.reconciled.push(input);
      if (failure.reconcile) throw new Error(failure.reconcile);
      return held.has(input.event.id) ? "held" : "absent";
    },
    outbox: createReviewOutbox(storage),
  };
});

function Harness({ revision }) {
  latest = useReviewComposer({
    revision,
    blocks: BLOCKS,
    executiveAgentPubkey: AGENT,
    parentEventId: NOTIFICATION_ID,
    rootEventId: ORIGIN_EVENT,
    blockedReason: null,
    measureViewport: () => ({ width: 1440, height: 900 }),
    focusFrameBlock: () => true,
    getScope: () => ({ ...scope }),
    onFeedbackSent: () => {
      log.sent += 1;
    },
    deps,
  });
  return null;
}

const mounted = [];

/** Mount a workbench-like host for one revision; `unmount` drops its state. */
async function mount(revision) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const view = {
    unmount: () =>
      act(async () => {
        root.unmount();
        container.remove();
      }),
  };
  mounted.push(view);
  await act(async () => {
    root.render(React.createElement(Harness, { revision }));
  });
  return view;
}

afterEach(async () => {
  for (const view of mounted.splice(0)) await view.unmount();
});

const openPrimary = () =>
  act(async () => {
    latest.open(PRIMARY, { kind: "element", element: document.body });
  });
const send = () =>
  act(async () => {
    await latest.submit();
  });

/** Type and send one comment; the wake fails, leaving feedback published. */
async function signWithFailedWake(view) {
  failure.wake = "wake failed";
  await openPrimary();
  await act(async () => latest.setText(COMMENT));
  await send();
  assert.equal(latest.phase.kind, "error");
  assert.equal(latest.feedbackPublished, true);
  assert.equal(log.published.length, 1);
  await view.unmount();
}

test("accepted feedback whose wake failed returns locked with Retry after a remount", async () => {
  await signWithFailedWake(await mount(revisionOne));

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 1);
  assert.equal(latest.pendingDeliveries[0].blockId, PRIMARY);
  assert.equal(latest.pendingDeliveries[0].feedbackPublished, true);
  assert.equal(latest.pendingDeliveries[0].feedbackEventId, log.signed[0].id);

  await openPrimary();
  assert.equal(latest.locked, true);
  assert.equal(latest.feedbackPublished, true);
  assert.equal(latest.text, COMMENT);

  // Retry finishes the same feedback: one signature, one publish, one wake.
  failure.wake = null;
  await send();
  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 1);
  assert.equal(log.notified.length, 1);
  assert.equal(log.notified[0].feedbackRevisionEventId, log.signed[0].id);
  assert.equal(latest.pendingDeliveries.length, 0);
  assert.equal(latest.activeBlockId, null);
  assert.equal(log.sent, 1);
});

test("a completed comment is forgotten and does not return on the next mount", async () => {
  const first = await mount(revisionOne);
  await openPrimary();
  await act(async () => latest.setText(COMMENT));
  await send();
  assert.equal(log.notified.length, 1);
  await first.unmount();

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 0);
  await openPrimary();
  assert.equal(latest.locked, false);
  assert.equal(latest.text, "");
});

test("a signed comment never carries into another revision, and returns on its own", async () => {
  await signWithFailedWake(await mount(revisionOne));

  const second = await mount(revisionTwo);
  assert.equal(latest.pendingDeliveries.length, 0);
  await openPrimary();
  assert.equal(latest.locked, false);
  assert.equal(latest.text, "");
  await second.unmount();

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 1);
});

test("a signed comment is only found under the relay and signer it was signed for", async () => {
  await signWithFailedWake(await mount(revisionOne));

  scope.relayUrl = OTHER_RELAY;
  const otherRelay = await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 0);
  await otherRelay.unmount();

  scope.relayUrl = RELAY;
  scope.signerPubkey = "c".repeat(64);
  const otherSigner = await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 0);
  await otherSigner.unmount();

  scope.signerPubkey = REVIEWER;
  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 1);
});

test("a restored comment fails closed when the captured community changes, and sends nothing", async () => {
  await signWithFailedWake(await mount(revisionOne));
  failure.wake = null;

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 1);
  // The community changes under the open workbench before Retry.
  scope.relayUrl = OTHER_RELAY;
  await openPrimary();
  await send();
  assert.equal(latest.phase.kind, "error");
  assert.match(latest.phase.message, /community or account changed/);
  assert.equal(log.notified.length, 0);
  assert.equal(log.published.length, 1);
  assert.equal(latest.pendingDeliveries.length, 1);

  // Back in the captured community the same comment finishes.
  scope.relayUrl = RELAY;
  await send();
  assert.equal(log.notified.length, 1);
  assert.equal(latest.pendingDeliveries.length, 0);
});

test("a published comment cannot be discarded; one the relay refused and holds nothing for can, for good", async () => {
  const first = await mount(revisionOne);
  failure.refusal =
    "conflict: feedback target is not the current review revision";
  await openPrimary();
  await act(async () => latest.setText(COMMENT));
  await send();
  assert.equal(latest.phase.kind, "error");
  assert.equal(latest.feedbackPublished, false);
  assert.equal(latest.locked, true);
  await first.unmount();

  const second = await mount(revisionOne);
  await openPrimary();
  await settle();
  assert.equal(latest.locked, true);
  assert.equal(latest.feedbackPublished, false);
  await act(async () => latest.discard());
  assert.equal(latest.locked, false);
  assert.equal(latest.text, COMMENT);
  assert.equal(latest.pendingDeliveries.length, 0);
  await second.unmount();

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 0);
});

test("discard leaves a published comment locked", async () => {
  await signWithFailedWake(await mount(revisionOne));
  await mount(revisionOne);
  await openPrimary();
  await act(async () => latest.discard());
  assert.equal(latest.locked, true);
  assert.equal(latest.pendingDeliveries.length, 1);
});

test("a comment that never reached the signer leaves nothing behind", async () => {
  const first = await mount(revisionOne);
  deps.signEvent = async () => {
    throw new Error("signer unavailable");
  };
  await openPrimary();
  await act(async () => latest.setText(COMMENT));
  await send();
  assert.equal(latest.phase.kind, "error");
  assert.equal(latest.locked, false);
  await first.unmount();

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 0);
});

test("a restored comment retries with the agent it was signed for, without a notification agent", async () => {
  await signWithFailedWake(await mount(revisionOne));
  failure.wake = null;

  // Reopened from history: no notification names an agent this time.
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  mounted.push({
    unmount: () =>
      act(async () => {
        root.unmount();
        container.remove();
      }),
  });
  function NoAgentHarness() {
    latest = useReviewComposer({
      revision: revisionOne,
      blocks: BLOCKS,
      executiveAgentPubkey: null,
      parentEventId: ORIGIN_EVENT,
      rootEventId: ORIGIN_EVENT,
      blockedReason: null,
      measureViewport: () => ({ width: 1, height: 1 }),
      focusFrameBlock: () => true,
      getScope: () => ({ ...scope }),
      onFeedbackSent: () => {
        log.sent += 1;
      },
      deps,
    });
    return null;
  }
  await act(async () => {
    root.render(React.createElement(NoAgentHarness));
  });
  assert.equal(latest.pendingDeliveries.length, 1);
  await openPrimary();
  await send();
  assert.equal(log.notified.length, 1);
  assert.equal(log.notified[0].executiveAgentPubkey, AGENT);
  assert.equal(log.notified[0].parentEventId, NOTIFICATION_ID);
});

test("the live scope guard follows whichever workbench is mounted and fails closed with none", async () => {
  const context = {
    expectedRelayUrl: RELAY,
    expectedSignerPubkey: REVIEWER,
  };
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), false);

  const first = await mount(revisionOne);
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), true);
  scope.relayUrl = OTHER_RELAY;
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), false);
  scope.relayUrl = RELAY;
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), true);

  // A submission signed in the first workbench must not keep reading its
  // props once it is gone.
  await first.unmount();
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), false);

  await mount(revisionOne);
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), true);
  scope.signerPubkey = "c".repeat(64);
  assert.equal(liveFeedbackDeps.isScopeCurrent(context), false);
});

// ── Ambiguous publishes, restarts, and the durable outbox ───────────────────

/** Let any background relay read finish and its result render. */
const settle = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

/** The comment is stored by the relay, but its answer is lost on the way. */
function loseAcknowledgement({ relayReadable }) {
  failure.publish = "relay unreachable: request timed out";
  failure.publishStored = true;
  failure.reconcile = relayReadable
    ? null
    : "relay unreachable: request timed out";
}

async function typeAndSend() {
  await openPrimary();
  await act(async () => latest.setText(COMMENT));
  await send();
}

test("a comment whose acknowledgement was lost cannot be discarded or replaced, only finished", async () => {
  const first = await mount(revisionOne);
  loseAcknowledgement({ relayReadable: false });
  await typeAndSend();
  assert.equal(latest.phase.kind, "error");
  assert.equal(latest.publishState, "ambiguous");
  assert.equal(latest.canDiscard, false);
  assert.equal(latest.pendingDeliveries[0].publishState, "ambiguous");
  assert.equal(latest.pendingDeliveries[0].canDiscard, false);

  // Discard changes nothing, and the signed text stays the signed text.
  await act(async () => latest.discard());
  assert.equal(latest.locked, true);
  assert.equal(latest.text, COMMENT);
  assert.equal(log.signed.length, 1);

  // The relay lists the comment it holds; it can never read as delivered.
  assert.deepEqual([...latest.unfinishedFeedbackEventIds], [log.signed[0].id]);
  await first.unmount();

  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries[0].publishState, "ambiguous");
  await openPrimary();
  assert.equal(latest.canDiscard, false);

  // The relay is readable again: Retry reads the exact event, adopts it, and
  // sends the one wake. No second signature, and nothing is resent.
  failure.publish = null;
  failure.reconcile = null;
  const publishCalls = log.publishCalls;
  await send();
  assert.equal(log.signed.length, 1);
  assert.equal(log.publishCalls, publishCalls, "the relay held it: no resend");
  assert.equal(log.notified.length, 1);
  assert.equal(log.notified[0].feedbackRevisionEventId, log.signed[0].id);
  assert.equal(latest.pendingDeliveries.length, 0);
  assert.equal(storage.data.size, 0);
});

test("a comment the relay turns out to hold is adopted on its own, so only its wake is owed", async () => {
  await mount(revisionOne);
  loseAcknowledgement({ relayReadable: true });
  await typeAndSend();
  await settle();
  assert.equal(latest.publishState, "accepted");
  assert.equal(latest.feedbackPublished, true);
  assert.equal(latest.canDiscard, false);
  assert.equal(log.notified.length, 0, "the wake is still owed");
  assert.deepEqual([...latest.unfinishedFeedbackEventIds], [log.signed[0].id]);
});

test("a restart keeps the comment: the outbox on disk resumes it with the same event and frozen wake", async () => {
  await signWithFailedWake(await mount(revisionOne));
  const [raw] = [...storage.data.values()];
  const savedWake = JSON.parse(raw).entries[0].wake;
  assert.equal(savedWake.feedbackRevisionEventId, log.signed[0].id);

  // Quit and relaunch: every live handle is gone; the outbox remains.
  resetSubmissionStore();
  failure.wake = null;
  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries.length, 1);
  assert.equal(latest.pendingDeliveries[0].publishState, "accepted");
  assert.deepEqual([...latest.unfinishedFeedbackEventIds], [log.signed[0].id]);
  await openPrimary();
  assert.equal(latest.locked, true);
  assert.equal(latest.text, COMMENT);

  await send();
  assert.equal(log.signed.length, 1);
  assert.equal(log.published.length, 1);
  assert.equal(log.notified.length, 1);
  assert.deepEqual(log.notified[0], savedWake);
  assert.equal(latest.pendingDeliveries.length, 0);
  assert.equal(storage.data.size, 0);
});

test("a restart after a lost acknowledgement resumes as ambiguous and finishes with one feedback and one wake", async () => {
  const view = await mount(revisionOne);
  loseAcknowledgement({ relayReadable: false });
  await typeAndSend();
  await view.unmount();

  resetSubmissionStore();
  await mount(revisionOne);
  assert.equal(latest.pendingDeliveries[0].publishState, "ambiguous");
  assert.equal(latest.pendingDeliveries[0].canDiscard, false);
  assert.deepEqual([...latest.unfinishedFeedbackEventIds], [log.signed[0].id]);

  failure.publish = null;
  failure.reconcile = null;
  await openPrimary();
  assert.equal(latest.locked, true);
  assert.equal(latest.canDiscard, false);
  await send();
  assert.equal(log.signed.length, 1);
  assert.equal(log.notified.length, 1);
  assert.equal(storage.data.size, 0);
});

test("Discard of a refused comment asks the relay first and fails closed when it cannot be read", async () => {
  await mount(revisionOne);
  failure.refusal =
    "conflict: feedback target is not the current review revision";
  await typeAndSend();
  assert.equal(latest.publishState, "rejected");
  assert.equal(latest.canDiscard, true);
  await settle();

  failure.reconcile = "relay unreachable: request timed out";
  await act(async () => latest.discard());
  assert.equal(latest.locked, true, "kept: the relay could not confirm");
  assert.equal(latest.phase.kind, "error");
  assert.match(latest.phase.message, /could not confirm/i);
  assert.equal(storage.data.size, 1);

  failure.reconcile = null;
  await act(async () => latest.discard());
  assert.equal(latest.locked, false);
  assert.equal(latest.text, COMMENT);
  assert.equal(latest.pendingDeliveries.length, 0);
  assert.equal(storage.data.size, 0);
});

test("a full outbox refuses a new comment with a reason, before anything is signed", async () => {
  await mount(revisionOne);
  // Occupy every slot with comments on other blocks, as a long outage would.
  const extra = createReviewOutbox(storage);
  const seeded = [];
  for (let index = 0; index < OUTBOX_MAX_ENTRIES; index += 1) {
    const block = {
      id: `other.${index}`,
      title: `Other ${index}`,
      sourceRef: null,
    };
    const submission = new ReviewFeedbackSubmission(
      {
        revision: revisionOne,
        block,
        request: COMMENT,
        viewport: { width: 1, height: 1 },
        executiveAgentPubkey: AGENT,
        parentEventId: NOTIFICATION_ID,
        rootEventId: ORIGIN_EVENT,
        expectedRelayUrl: RELAY,
        expectedSignerPubkey: REVIEWER,
      },
      {
        ...deps,
        outbox: extra,
        newFeedbackId: () => crypto.randomUUID(),
        publishEvent: async () => {
          throw new Error("relay unreachable: request timed out");
        },
      },
    );
    await assert.rejects(submission.submit());
    seeded.push(submission);
  }
  const signedBefore = log.signed.length;
  await openPrimary();
  await act(async () => latest.setText(COMMENT));
  await send();
  assert.equal(latest.phase.kind, "error");
  assert.match(latest.phase.message, /at most \d+ unfinished comments/);
  assert.equal(log.signed.length, signedBefore, "nothing was signed");
  assert.equal(latest.locked, false);
});

// ── Other revisions and the discarding phase ────────────────────────────────

test("an unfinished comment on another revision stays discoverable from the open one, and is never taken for its own", async () => {
  await signWithFailedWake(await mount(revisionOne));

  // Only the relay and signer it was signed for can see it.
  scope.relayUrl = OTHER_RELAY;
  const foreign = await mount(revisionTwo);
  assert.equal(latest.otherRevisionDeliveries.length, 0);
  await foreign.unmount();
  scope.relayUrl = RELAY;

  // After a restart the durable outbox, not a live handle, is what lists it.
  resetSubmissionStore();
  const second = await mount(revisionTwo);
  assert.equal(latest.pendingDeliveries.length, 0);
  assert.equal(latest.otherRevisionDeliveries.length, 1);
  const [other] = latest.otherRevisionDeliveries;
  assert.equal(other.revisionEventId, revisionOne.eventId);
  assert.equal(other.revisionNumber, revisionOne.revision);
  assert.equal(other.blockId, PRIMARY);
  assert.equal(other.blockTitle, BLOCKS[1].title);
  assert.equal(other.publishState, "accepted");
  assert.equal(other.feedbackEventId, log.signed[0].id);
  // Its wake is still owed, so the relay's listing cannot read it as delivered.
  assert.deepEqual([...latest.unfinishedFeedbackEventIds], [log.signed[0].id]);

  // It is not this revision's: the same block opens unlocked and empty.
  await openPrimary();
  assert.equal(latest.locked, false);
  assert.equal(latest.text, "");

  // A comment on this revision's own block is listed once, in its own list.
  failure.wake = "wake failed";
  await act(async () => latest.setText("A comment on the new revision."));
  await send();
  assert.equal(latest.pendingDeliveries.length, 1);
  assert.equal(latest.pendingDeliveries[0].feedbackEventId, log.signed[1].id);
  assert.equal(latest.otherRevisionDeliveries.length, 1);
  assert.equal(
    latest.otherRevisionDeliveries[0].feedbackEventId,
    log.signed[0].id,
  );
  await second.unmount();

  // Seen from revision one, the roles swap; nothing is dropped or duplicated.
  await mount(revisionOne);
  assert.deepEqual(
    latest.pendingDeliveries.map((delivery) => delivery.feedbackEventId),
    [log.signed[0].id],
  );
  assert.deepEqual(
    latest.otherRevisionDeliveries.map((delivery) => [
      delivery.revisionEventId,
      delivery.feedbackEventId,
    ]),
    [[revisionTwo.eventId, log.signed[1].id]],
  );

  // Finishing it on its own revision removes it from every view.
  failure.wake = null;
  await openPrimary();
  assert.equal(latest.locked, true);
  await send();
  assert.equal(latest.pendingDeliveries.length, 0);
  assert.deepEqual([...latest.unfinishedFeedbackEventIds], [log.signed[1].id]);
  assert.equal(latest.otherRevisionDeliveries.length, 1);
});

test("Discard is its own phase while the relay is asked, and nothing else runs until it settles", async () => {
  await mount(revisionOne);
  failure.refusal =
    "conflict: feedback target is not the current review revision";
  await typeAndSend();
  assert.equal(latest.publishState, "rejected");
  await settle();

  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const reads = [];
  deps.reconcileFeedback = async (input) => {
    reads.push(input);
    await gate;
    return "absent";
  };
  const publishCalls = log.publishCalls;
  let discarding;
  await act(async () => {
    discarding = latest.discard();
  });
  assert.equal(latest.phase.kind, "discarding");
  assert.equal(latest.locked, true, "still the signed comment until settled");

  // A second discard and a submit are ignored and leave the phase truthful.
  await act(async () => {
    await latest.discard();
    await latest.submit();
  });
  assert.equal(latest.phase.kind, "discarding");
  assert.equal(reads.length, 1);
  assert.equal(log.publishCalls, publishCalls);
  assert.equal(log.signed.length, 1);

  await act(async () => {
    release();
    await discarding;
  });
  assert.equal(latest.phase.kind, "idle");
  assert.equal(latest.locked, false);
  assert.equal(latest.text, COMMENT);
  assert.equal(storage.data.size, 0);
});

test("a discard the relay contradicts ends the discarding phase and keeps the comment", async () => {
  await mount(revisionOne);
  failure.refusal =
    "conflict: feedback target is not the current review revision";
  await typeAndSend();
  await settle();
  // The relay in fact stores it.
  held.add(log.signed[0].id);

  await act(async () => latest.discard());
  assert.equal(latest.phase.kind, "error");
  assert.equal(latest.locked, true);
  assert.equal(latest.publishState, "accepted");
  assert.equal(latest.canDiscard, false);
  assert.equal(storage.data.size, 1);
});
