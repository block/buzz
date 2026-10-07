/**
 * Review Canvas feedback chrome, rendered for real.
 *
 * Contract: accepted feedback whose agent notification is still owed is never
 * shown as an ordinary comment, a pending comment can be reopened from a real
 * button, and a listing the relay cut short says so.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import {
  bindFeedbackToReview,
  parseReviewFeedback,
  parseReviewRevision,
} from "../lib/reviewContract.ts";
import { feedbackEvent, reviewEvent } from "../lib/reviewFixtures.mjs";
import { ReviewFeedbackPanel } from "./ReviewFeedbackPanel.tsx";
import { ReviewPendingDelivery } from "./ReviewPendingDelivery.tsx";

const BLOCKS = [
  {
    id: "checkout.primary-action",
    title: "Primary checkout action",
    sourceRef: null,
  },
];

const revision = parseReviewRevision(reviewEvent());
const feedback = parseReviewFeedback(feedbackEvent({ revision }));
const item = {
  feedback,
  block: bindFeedbackToReview(feedback, revision, BLOCKS),
  revision,
};

const mounted = [];

async function render(element) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  mounted.push(() =>
    act(async () => {
      root.unmount();
      container.remove();
    }),
  );
  await act(async () => {
    root.render(element);
  });
  return container;
}

afterEach(async () => {
  for (const unmount of mounted.splice(0)) await unmount();
});

function panel(props = {}) {
  return React.createElement(ReviewFeedbackPanel, {
    revision,
    result: { items: [item], withheld: 0, truncated: false },
    loading: false,
    failed: false,
    currentPubkey: undefined,
    awaitingNotification: new Set(),
    ...props,
  });
}

test("accepted feedback awaiting its agent notification is marked, not shown as an ordinary comment", async () => {
  const container = await render(
    panel({ awaitingNotification: new Set([feedback.eventId]) }),
  );
  const pending = container.querySelector(
    '[data-testid="review-feedback-pending"]',
  );
  assert.equal(pending?.textContent, "Agent not notified yet");
  assert.doesNotMatch(container.textContent, /Awaiting revision/);
});

test("feedback with no owed notification keeps its ordinary badge", async () => {
  const container = await render(panel());
  assert.equal(
    container.querySelector('[data-testid="review-feedback-pending"]'),
    null,
  );
  assert.match(container.textContent, /Awaiting revision/);
});

test("a listing cut short by the relay page cap says so", async () => {
  const cut = await render(
    panel({ result: { items: [item], withheld: 0, truncated: true } }),
  );
  assert.match(
    cut.querySelector('[data-testid="review-feedback-truncated"]')
      ?.textContent ?? "",
    /oldest comments are not shown/,
  );

  const whole = await render(panel());
  assert.equal(
    whole.querySelector('[data-testid="review-feedback-truncated"]'),
    null,
  );
});

const delivery = (overrides) => ({
  blockId: "checkout.primary-action",
  blockTitle: "Primary checkout action",
  feedbackPublished: true,
  publishState: "accepted",
  canDiscard: false,
  feedbackEventId: feedback.eventId,
  ...overrides,
});

function pendingNotice(props = {}) {
  return React.createElement(ReviewPendingDelivery, {
    deliveries: [],
    otherRevisionDeliveries: [],
    onOpen: () => {},
    onOpenRevision: () => {},
    ...props,
  });
}

const openButton = (container, blockId) =>
  container.querySelector(`[data-testid="review-pending-open-${blockId}"]`);

test("pending deliveries open their comment from real buttons", async () => {
  const opened = [];
  const navigated = [];
  const container = await render(
    pendingNotice({
      deliveries: [
        delivery({}),
        delivery({
          blockId: "checkout.summary",
          blockTitle: "Order summary",
          feedbackPublished: false,
          publishState: "ambiguous",
          feedbackEventId: null,
        }),
      ],
      onOpen: (blockId) => opened.push(blockId),
      onOpenRevision: (eventId) => navigated.push(eventId),
    }),
  );
  assert.equal(container.querySelectorAll("button").length, 2);
  for (const blockId of ["checkout.primary-action", "checkout.summary"]) {
    assert.equal(openButton(container, blockId)?.tagName, "BUTTON");
  }

  await act(async () => {
    openButton(container, "checkout.summary").click();
  });
  assert.deepEqual(opened, ["checkout.summary"]);
  assert.deepEqual(navigated, []);
});

test("an unfinished comment on another revision names it and goes to exactly that revision, never opening as this revision's", async () => {
  const EARLIER = "1".repeat(64);
  const LATER = "3".repeat(64);
  const opened = [];
  const navigated = [];
  const container = await render(
    pendingNotice({
      // The same block has a comment on this revision and on another one.
      deliveries: [delivery({})],
      otherRevisionDeliveries: [
        delivery({ revisionEventId: LATER, revisionNumber: 3 }),
        delivery({
          blockId: "checkout.summary",
          blockTitle: "Order summary",
          revisionEventId: EARLIER,
          revisionNumber: 1,
          feedbackPublished: false,
          publishState: "rejected",
          canDiscard: true,
          feedbackEventId: null,
        }),
      ],
      onOpen: (blockId) => opened.push(blockId),
      onOpenRevision: (eventId) => navigated.push(eventId),
    }),
  );
  const goTo = (eventId, blockId) =>
    container.querySelector(
      `[data-testid="review-pending-revision-${eventId}-${blockId}"]`,
    );
  const earlier = goTo(EARLIER, "checkout.summary");
  const later = goTo(LATER, "checkout.primary-action");

  // Real buttons, each in an entry that states its revision and block.
  assert.equal(earlier?.tagName, "BUTTON");
  assert.equal(later?.tagName, "BUTTON");
  const earlierEntry = earlier.closest("li").textContent;
  assert.match(earlierEntry, /Order summary/);
  assert.match(earlierEntry, /\b1\b/);
  const laterEntry = later.closest("li").textContent;
  assert.match(laterEntry, /Primary checkout action/);
  assert.match(laterEntry, /\b3\b/);
  // Three entries: this revision's own, plus one per other revision.
  assert.equal(container.querySelectorAll("li").length, 3);
  assert.equal(container.querySelectorAll("button").length, 3);

  await act(async () => {
    earlier.click();
  });
  assert.deepEqual(navigated, [EARLIER]);
  assert.deepEqual(opened, [], "it is not opened as a form on this revision");

  await act(async () => {
    later.click();
    openButton(container, "checkout.primary-action").click();
  });
  assert.deepEqual(navigated, [EARLIER, LATER]);
  assert.deepEqual(opened, ["checkout.primary-action"]);
});

test("a comment pending only on another revision still shows the notice", async () => {
  const container = await render(
    pendingNotice({
      otherRevisionDeliveries: [
        delivery({ revisionEventId: "1".repeat(64), revisionNumber: 1 }),
      ],
    }),
  );
  assert.notEqual(
    container.querySelector('[data-testid="review-pending-delivery"]'),
    null,
  );
  assert.equal(container.querySelectorAll("button").length, 1);
});

test("nothing renders when no delivery is pending", async () => {
  const container = await render(pendingNotice());
  assert.equal(container.innerHTML, "");
});
