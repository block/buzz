import assert from "node:assert/strict";
import test from "node:test";

import {
  findFeedbackChannel,
  normalizeFeedbackChannelId,
} from "./useSendFeedback.ts";

const FEEDBACK_CHANNEL_ID = "configured-feedback-channel";

/** Minimal Channel-shaped fixture for configured destination matching. */
function channel(overrides) {
  return {
    id: "other-channel",
    name: "general",
    channelType: "stream",
    visibility: "open",
    isMember: true,
    archivedAt: null,
    ...overrides,
  };
}

/** An active private stream matching the configured channel ID. */
function feedbackChannel(overrides) {
  return channel({
    id: FEEDBACK_CHANNEL_ID,
    name: "Support inbox",
    visibility: "private",
    isMember: true,
    ...overrides,
  });
}

test("normalizeFeedbackChannelId_returnsNullWhenUnsetOrBlank", () => {
  assert.equal(normalizeFeedbackChannelId(undefined), null);
  assert.equal(normalizeFeedbackChannelId(""), null);
  assert.equal(normalizeFeedbackChannelId("   "), null);
});

test("normalizeFeedbackChannelId_trimsConfiguredId", () => {
  assert.equal(
    normalizeFeedbackChannelId(`  ${FEEDBACK_CHANNEL_ID}  `),
    FEEDBACK_CHANNEL_ID,
  );
});

test("findFeedbackChannel_returnsNullWhenUndefined", () => {
  assert.equal(findFeedbackChannel(undefined, FEEDBACK_CHANNEL_ID), null);
});

test("findFeedbackChannel_matchesExactConfiguredId", () => {
  const match = feedbackChannel();
  assert.equal(
    findFeedbackChannel([channel(), match], FEEDBACK_CHANNEL_ID),
    match,
  );
});

test("findFeedbackChannel_doesNotFallBackToName", () => {
  const sameName = feedbackChannel({ id: "wrong-id" });
  assert.equal(findFeedbackChannel([sameName], FEEDBACK_CHANNEL_ID), null);
});

test("findFeedbackChannel_excludesDmChannels", () => {
  const dm = feedbackChannel({ channelType: "dm" });
  assert.equal(findFeedbackChannel([dm], FEEDBACK_CHANNEL_ID), null);
});

test("findFeedbackChannel_excludesOpenChannels", () => {
  const open = feedbackChannel({ visibility: "open" });
  assert.equal(findFeedbackChannel([open], FEEDBACK_CHANNEL_ID), null);
});

test("findFeedbackChannel_excludesNonMemberChannels", () => {
  const notMember = feedbackChannel({ isMember: false });
  assert.equal(findFeedbackChannel([notMember], FEEDBACK_CHANNEL_ID), null);
});

test("findFeedbackChannel_excludesForumChannels", () => {
  const forum = feedbackChannel({ channelType: "forum" });
  assert.equal(findFeedbackChannel([forum], FEEDBACK_CHANNEL_ID), null);
});

test("findFeedbackChannel_excludesArchivedChannels", () => {
  const archived = feedbackChannel({
    archivedAt: "2026-01-01T00:00:00Z",
  });
  assert.equal(findFeedbackChannel([archived], FEEDBACK_CHANNEL_ID), null);
});
