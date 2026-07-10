import assert from "node:assert/strict";
import test from "node:test";

import { settleProgrammaticBottomPin } from "./useAnchoredScroll.ts";
import { computeAnchorCorrection } from "./useAnchoredScroll.ts";

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

// computeAnchorCorrection — the scroll-invariant realization compensation.
// Convention: scrollTop grows downward; a row's topOffset is its top relative
// to the viewport top. A reflow ABOVE the row pushes it down (topOffset grows)
// at constant scrollTop; a user scroll down grows scrollTop and shrinks
// topOffset by equal amounts (document position fixed).

test("computeAnchorCorrection returns null when nothing shifted", () => {
  const anchor = { topOffset: 100, scrollTop: 500 };
  assert.equal(computeAnchorCorrection(anchor, anchor), null);
});

test("computeAnchorCorrection ignores pure user scroll (no reflow)", () => {
  // User scrolled DOWN 40px since baseline: scrollTop +40, topOffset -40.
  const baseline = { topOffset: 100, scrollTop: 500 };
  const current = { topOffset: 60, scrollTop: 540 };
  // Document position unchanged => no correction, the wheel is left alone.
  assert.equal(computeAnchorCorrection(baseline, current), null);
});

test("computeAnchorCorrection compensates a reflow above the row, scrolling down by the shift", () => {
  // A row above realized 30px taller: at constant scrollTop the anchor's
  // topOffset grew 100 -> 130. To keep it visually fixed, scroll down 30px.
  const baseline = { topOffset: 100, scrollTop: 500 };
  const current = { topOffset: 130, scrollTop: 500 };
  assert.equal(computeAnchorCorrection(baseline, current), 530);
});

test("computeAnchorCorrection compensates a reflow that shrank content above", () => {
  // Content above shrank 20px: topOffset 100 -> 80 at constant scrollTop.
  // Correct by scrolling UP 20px so the row stays put.
  const baseline = { topOffset: 100, scrollTop: 500 };
  const current = { topOffset: 80, scrollTop: 500 };
  assert.equal(computeAnchorCorrection(baseline, current), 480);
});

test("computeAnchorCorrection isolates reflow from a simultaneous user scroll", () => {
  // Since baseline the user scrolled down 40px (scrollTop +40, topOffset -40)
  // AND a row above realized 30px taller (topOffset +30). Net topOffset:
  // 100 - 40 + 30 = 90; scrollTop 540. Only the 30px reflow should be
  // compensated: target = 540 + 30 = 570, leaving the user's 40px intact.
  const baseline = { topOffset: 100, scrollTop: 500 };
  const current = { topOffset: 90, scrollTop: 540 };
  assert.equal(computeAnchorCorrection(baseline, current), 570);
});

test("computeAnchorCorrection treats sub-epsilon shift as noise", () => {
  const baseline = { topOffset: 100, scrollTop: 500 };
  const current = { topOffset: 100.3, scrollTop: 500 };
  assert.equal(computeAnchorCorrection(baseline, current), null);
});
