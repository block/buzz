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
    visibility: "public",
    ...overrides,
  };
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
  const match = channel({ id: "fb", name: "buzz FEEDBACK" });
  assert.equal(findFeedbackChannel([channel(), match]), match);
});

test("findFeedbackChannel_trimsSurroundingWhitespace", () => {
  const match = channel({ id: "fb", name: `  ${FEEDBACK_CHANNEL_NAME}  ` });
  assert.equal(findFeedbackChannel([match]), match);
});

test("findFeedbackChannel_excludesDmChannelsWithMatchingName", () => {
  const dm = channel({
    id: "dm",
    name: FEEDBACK_CHANNEL_NAME,
    channelType: "dm",
  });
  assert.equal(findFeedbackChannel([dm]), null);
});
