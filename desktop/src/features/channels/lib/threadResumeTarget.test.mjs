import assert from "node:assert/strict";
import test from "node:test";

import { selectThreadResumeTargetId } from "./threadResumeTarget.ts";

// Every gate defaults open so each test can close exactly one of them.
function input(overrides = {}) {
  return {
    openThreadHeadId: "head-1",
    firstUnreadReplyId: "reply-7",
    externalScrollTargetId: null,
    routeTargetMessageId: null,
    hasReplies: true,
    isHuddleTranscript: false,
    ...overrides,
  };
}

test("resumes at the first unread reply when nothing outranks it", () => {
  assert.equal(selectThreadResumeTargetId(input()), "reply-7");
});

test("a fully read thread keeps today's bottom pin", () => {
  assert.equal(
    selectThreadResumeTargetId(input({ firstUnreadReplyId: null })),
    null,
  );
});

test("no resume while the panel is closed", () => {
  assert.equal(
    selectThreadResumeTargetId(input({ openThreadHeadId: null })),
    null,
  );
});

test("a route deep link outranks the resume", () => {
  assert.equal(
    selectThreadResumeTargetId(input({ routeTargetMessageId: "reply-2" })),
    null,
  );
});

test("a panel deep link outranks the resume", () => {
  assert.equal(
    selectThreadResumeTargetId(input({ externalScrollTargetId: "reply-3" })),
    null,
  );
});

test("does not latch before the first reply has rendered", () => {
  // Latching here would freeze a null decision for the whole open, and the
  // real marker arrives a render or two later.
  assert.equal(selectThreadResumeTargetId(input({ hasReplies: false })), null);
});

test("huddle transcripts are a replay surface, never resumed", () => {
  assert.equal(
    selectThreadResumeTargetId(input({ isHuddleTranscript: true })),
    null,
  );
});

test("a nested (submessage) thread head resumes on the same rule", () => {
  // The panel passes whichever head is open; a nested head is not special.
  assert.equal(
    selectThreadResumeTargetId(
      input({ openThreadHeadId: "nested-head", firstUnreadReplyId: "n-3" }),
    ),
    "n-3",
  );
});
