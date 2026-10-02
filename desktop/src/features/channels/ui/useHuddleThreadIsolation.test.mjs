import assert from "node:assert/strict";
import test from "node:test";

import { resolveHuddleOpenThreadHeadId } from "./useHuddleThreadIsolation.ts";

test("huddle transcripts synchronously hide URL thread state", () => {
  assert.equal(
    resolveHuddleOpenThreadHeadId({
      isHuddleTranscript: true,
      openThreadHeadId: "url-thread",
      optimisticOpenThreadHeadId: undefined,
    }),
    null,
  );
});

test("an optimistic null overrides the URL thread until navigation settles", () => {
  assert.equal(
    resolveHuddleOpenThreadHeadId({
      isHuddleTranscript: false,
      openThreadHeadId: "url-thread",
      optimisticOpenThreadHeadId: null,
    }),
    null,
  );
  assert.equal(
    resolveHuddleOpenThreadHeadId({
      isHuddleTranscript: false,
      openThreadHeadId: "url-thread",
      optimisticOpenThreadHeadId: undefined,
    }),
    "url-thread",
  );
});
