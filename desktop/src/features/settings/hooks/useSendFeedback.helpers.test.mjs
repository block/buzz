import assert from "node:assert/strict";
import test from "node:test";

import {
  FEEDBACK_CHANNEL_NAME,
  findFeedbackChannel,
} from "./useSendFeedback.ts";

/** Minimal Channel-shaped fixture for the name/type match logic. */
function channel(overrides) {
  return {
    id: "id",
    name: "general",
    channelType: "stream",
    visibility: "open",
    isMember: true,
    ...overrides,
  };
}

/** A private channel the user is a member of, named like the feedback channel. */
function feedbackChannel(overrides) {
  return channel({
    id: "fb",
    name: FEEDBACK_CHANNEL_NAME,
    visibility: "private",
    isMember: true,
    ...overrides,
  });
}

test("findFeedbackChannel_returnsNullWhenUndefined", () => {
  assert.equal(findFeedbackChannel(undefined), null);
});

test("findFeedbackChannel_returnsNullWhenNoMatch", () => {
  assert.equal(
    findFeedbackChannel([
      channel({ name: "general" }),
      channel({ name: "random" }),
    ]),
    null,
  );
});

test("findFeedbackChannel_matchesByNameCaseInsensitively", () => {
  const match = feedbackChannel({ name: "buzz FEEDBACK" });
  assert.equal(findFeedbackChannel([channel(), match]), match);
});

test("findFeedbackChannel_trimsSurroundingWhitespace", () => {
  const match = feedbackChannel({ name: `  ${FEEDBACK_CHANNEL_NAME}  ` });
  assert.equal(findFeedbackChannel([match]), match);
});

test("findFeedbackChannel_excludesDmChannelsWithMatchingName", () => {
  const dm = feedbackChannel({ id: "dm", channelType: "dm" });
  assert.equal(findFeedbackChannel([dm]), null);
});

test("findFeedbackChannel_excludesOpenChannelsWithMatchingName", () => {
  // An open (public) channel sharing the name must NOT be reused — feedback
  // would leak publicly. Only a private channel qualifies.
  const open = feedbackChannel({ id: "open", visibility: "open" });
  assert.equal(findFeedbackChannel([open]), null);
});

test("findFeedbackChannel_excludesNonMemberChannelsWithMatchingName", () => {
  // A private channel the user is not a member of can't receive the message,
  // and reusing it would be wrong — skip it and create a fresh one.
  const notMember = feedbackChannel({ id: "nm", isMember: false });
  assert.equal(findFeedbackChannel([notMember]), null);
});
