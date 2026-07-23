import assert from "node:assert/strict";
import test from "node:test";

import {
  getPinnedCenterDrift,
  settleProgrammaticBottomPin,
  shouldIgnorePinnedCenterScroll,
  shouldSettleForSplitPanel,
  shouldSettleVirtualizedBottom,
} from "./useAnchoredScroll.ts";

function fakeContainer({ clientHeight, scrollHeight, scrollTop }) {
  const writes = [];
  return {
    clientHeight,
    scrollHeight,
    scrollTop,
    writes,
    scrollTo({ top, behavior }) {
      writes.push({ top, behavior });
      this.scrollTop = top;
    },
  };
}

test("split panel settles only an already-bottomed timeline", () => {
  assert.equal(
    shouldSettleForSplitPanel({ isAtBottom: true, splitPanelOpen: true }),
    true,
  );
  assert.equal(
    shouldSettleForSplitPanel({ isAtBottom: false, splitPanelOpen: true }),
    false,
  );
  assert.equal(
    shouldSettleForSplitPanel({ isAtBottom: true, splitPanelOpen: false }),
    false,
  );
});

test("virtualized bottom settle arms for pinned appends and replacements", () => {
  assert.equal(
    shouldSettleVirtualizedBottom({
      isAtBottom: true,
      messageDelta: "append",
      messagesArrived: 1,
      messagesChanged: true,
    }),
    true,
  );
  assert.equal(
    shouldSettleVirtualizedBottom({
      isAtBottom: true,
      messageDelta: "replace",
      messagesArrived: 0,
      messagesChanged: true,
    }),
    true,
  );
  assert.equal(
    shouldSettleVirtualizedBottom({
      isAtBottom: true,
      messageDelta: "none",
      messagesArrived: 0,
      messagesChanged: true,
    }),
    true,
  );
  assert.equal(
    shouldSettleVirtualizedBottom({
      isAtBottom: true,
      messageDelta: "prepend",
      messagesArrived: 1,
      messagesChanged: true,
    }),
    false,
  );
  assert.equal(
    shouldSettleVirtualizedBottom({
      isAtBottom: true,
      messageDelta: "none",
      messagesArrived: 0,
      messagesChanged: false,
    }),
    false,
  );
  assert.equal(
    shouldSettleVirtualizedBottom({
      isAtBottom: false,
      messageDelta: "none",
      messagesArrived: 0,
      messagesChanged: true,
    }),
    false,
  );
});

test("settleProgrammaticBottomPin chases the physical floor before clearing", () => {
  const container = fakeContainer({
    clientHeight: 100,
    scrollHeight: 200,
    scrollTop: 70,
  });

  assert.equal(settleProgrammaticBottomPin(container), true);
  assert.deepEqual(container.writes, [{ top: 200, behavior: "auto" }]);
  assert.equal(container.scrollTop, 200);
});

test("settleProgrammaticBottomPin keeps settling when the floor is still out of reach", () => {
  const container = fakeContainer({
    clientHeight: 100,
    scrollHeight: 200,
    scrollTop: 70,
  });
  container.scrollTo = ({ top, behavior }) => {
    container.writes.push({ top, behavior });
    // Browser/virtualizer has not caught up yet: leave a >1px physical gap.
    container.scrollTop = 98;
  };

  assert.equal(settleProgrammaticBottomPin(container), false);
  assert.deepEqual(container.writes, [{ top: 200, behavior: "auto" }]);
  assert.equal(
    container.scrollHeight - container.clientHeight - container.scrollTop,
    2,
  );
});

test("pinned center drift re-pins only after meaningful layout growth", () => {
  assert.equal(
    getPinnedCenterDrift({ contentTop: 400, currentContentTop: 400.5 }),
    null,
  );
  assert.equal(
    getPinnedCenterDrift({ contentTop: 400, currentContentTop: 496 }),
    96,
  );
});

test("pinned center programmatic scroll event preserves the anchor", () => {
  assert.equal(
    shouldIgnorePinnedCenterScroll({
      currentScrollTop: 596,
      expectedScrollTop: 596,
      isWritingScroll: false,
    }),
    true,
  );
  assert.equal(
    shouldIgnorePinnedCenterScroll({
      currentScrollTop: 596,
      expectedScrollTop: null,
      isWritingScroll: true,
    }),
    true,
  );
});

test("pinned center real user scroll releases the anchor", () => {
  assert.equal(
    shouldIgnorePinnedCenterScroll({
      currentScrollTop: 620,
      expectedScrollTop: 596,
      isWritingScroll: false,
    }),
    false,
  );
});
