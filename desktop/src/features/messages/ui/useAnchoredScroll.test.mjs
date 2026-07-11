import assert from "node:assert/strict";
import test from "node:test";

import {
  settleProgrammaticBottomPin,
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

test("virtualized bottom settle arms only for pinned appends", () => {
  assert.equal(
    shouldSettleVirtualizedBottom({
      anchorKind: "at-bottom",
      messageDelta: "append",
      messagesArrived: 1,
    }),
    true,
  );
  for (const messageDelta of ["prepend", "replace", "none"]) {
    assert.equal(
      shouldSettleVirtualizedBottom({
        anchorKind: "at-bottom",
        messageDelta,
        messagesArrived: 1,
      }),
      false,
    );
  }
  assert.equal(
    shouldSettleVirtualizedBottom({
      anchorKind: "message",
      messageDelta: "append",
      messagesArrived: 1,
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
