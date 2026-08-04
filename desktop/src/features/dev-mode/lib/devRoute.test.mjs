import assert from "node:assert/strict";
import test from "node:test";

import { isSettlingTransition, readDevRouteSeed } from "./devRoute.ts";

function selection(view, channelId = null, threadRootId = null) {
  return { view, channelId, threadRootId };
}

test("readDevRouteSeed_readsChannelPath", () => {
  assert.deepEqual(
    readDevRouteSeed({ pathname: "/channels/abc-123", search: {} }),
    { channelId: "abc-123", threadRootId: null },
  );
});

test("readDevRouteSeed_readsOpenThreadParam", () => {
  assert.deepEqual(
    readDevRouteSeed({
      pathname: "/channels/abc",
      search: { thread: "root1" },
    }),
    { channelId: "abc", threadRootId: "root1" },
  );
});

test("readDevRouteSeed_fallsBackToDeepLinkThreadRootId", () => {
  assert.deepEqual(
    readDevRouteSeed({
      pathname: "/channels/abc",
      search: { threadRootId: "root2" },
    }),
    { channelId: "abc", threadRootId: "root2" },
  );
});

test("readDevRouteSeed_prefersThreadOverThreadRootId", () => {
  assert.equal(
    readDevRouteSeed({
      pathname: "/channels/abc",
      search: { thread: "root1", threadRootId: "root2" },
    }).threadRootId,
    "root1",
  );
});

test("readDevRouteSeed_ignoresNonStringOrEmptyThread", () => {
  assert.equal(
    readDevRouteSeed({ pathname: "/channels/abc", search: { thread: "" } })
      .threadRootId,
    null,
  );
  assert.equal(
    readDevRouteSeed({ pathname: "/channels/abc", search: { thread: 7 } })
      .threadRootId,
    null,
  );
});

test("readDevRouteSeed_rejectsNonChannelPaths", () => {
  assert.equal(readDevRouteSeed({ pathname: "/", search: {} }), null);
  assert.equal(readDevRouteSeed({ pathname: "/projects", search: {} }), null);
  assert.equal(
    readDevRouteSeed({ pathname: "/channels/abc/posts/p1", search: {} }),
    null,
  );
});

const seed = { channelId: "chan", threadRootId: "root" };

test("isSettlingTransition_channelResolvingTowardSeed", () => {
  assert.ok(
    isSettlingTransition(
      selection("channel"),
      selection("channel", "chan"),
      seed,
    ),
  );
});

test("isSettlingTransition_threadResolvingTowardSeed", () => {
  assert.ok(
    isSettlingTransition(
      selection("channel", "chan"),
      selection("channel", "chan", "root"),
      seed,
    ),
  );
});

test("isSettlingTransition_userSwitchingChannelIsDirty", () => {
  assert.ok(
    !isSettlingTransition(
      selection("channel", "chan"),
      selection("channel", "other"),
      seed,
    ),
  );
});

test("isSettlingTransition_closingSeededThreadIsDirty", () => {
  assert.ok(
    !isSettlingTransition(
      selection("channel", "chan", "root"),
      selection("channel", "chan"),
      seed,
    ),
  );
});

test("isSettlingTransition_openingDifferentThreadIsDirty", () => {
  assert.ok(
    !isSettlingTransition(
      selection("channel", "chan"),
      selection("channel", "chan", "other-root"),
      seed,
    ),
  );
});

test("isSettlingTransition_leavingChannelViewIsDirty", () => {
  assert.ok(
    !isSettlingTransition(
      selection("channel", "chan"),
      selection("navigator"),
      seed,
    ),
  );
});

test("isSettlingTransition_withoutSeedEverythingIsDirty", () => {
  assert.ok(
    !isSettlingTransition(
      selection("channel"),
      selection("channel", "chan"),
      null,
    ),
  );
});
