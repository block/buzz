import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { buildTimeMap } from "./activityTime.ts";

function expectClose(actual, expected) {
  assert.ok(
    Math.abs(actual - expected) < 10 ** -8,
    `expected ${actual} to be close to ${expected}`,
  );
}

describe("buildTimeMap", () => {
  it("is identity-like when no gaps exceed the threshold", () => {
    const times = [1_000, 6_000, 16_000, 26_000];
    const map = buildTimeMap(times);

    assert.deepEqual(map.gaps, []);
    assert.equal(map.displayDuration, 25_000);
    assert.equal(map.toDisplay(1_000), 0);
    assert.equal(map.toDisplay(11_000), 10_000);
    assert.equal(map.toReal(10_000), 11_000);
  });

  it("compresses a single gap to the configured display duration", () => {
    const map = buildTimeMap([1_000, 11_000, 81_000, 91_000], {
      gapThresholdMs: 30_000,
      gapDisplayMs: 2_500,
    });

    assert.deepEqual(map.gaps, [
      {
        realStart: 11_000,
        realEnd: 81_000,
        displayStart: 10_000,
        displayEnd: 12_500,
      },
    ]);
    assert.equal(map.displayDuration, 22_500);
    assert.equal(map.toDisplay(81_000), 12_500);
    assert.equal(map.toReal(12_500), 81_000);
    expectClose(map.toDisplay(46_000), 11_250);
    expectClose(map.toReal(11_250), 46_000);
  });

  it("compresses multiple gaps", () => {
    const map = buildTimeMap([0, 10_000, 70_000, 80_000, 200_000], {
      gapThresholdMs: 30_000,
      gapDisplayMs: 2_500,
    });

    assert.deepEqual(map.gaps, [
      {
        realStart: 10_000,
        realEnd: 70_000,
        displayStart: 10_000,
        displayEnd: 12_500,
      },
      {
        realStart: 80_000,
        realEnd: 200_000,
        displayStart: 22_500,
        displayEnd: 25_000,
      },
    ]);
    assert.equal(map.displayDuration, 25_000);
    assert.equal(map.toDisplay(75_000), 17_500);
    assert.equal(map.toReal(23_750), 140_000);
  });

  it("does not compress an interval exactly at the threshold", () => {
    const map = buildTimeMap([0, 30_000, 60_001], {
      gapThresholdMs: 30_000,
      gapDisplayMs: 2_500,
    });

    assert.deepEqual(map.gaps, [
      {
        realStart: 30_000,
        realEnd: 60_001,
        displayStart: 30_000,
        displayEnd: 32_500,
      },
    ]);
    assert.equal(map.toDisplay(30_000), 30_000);
    assert.equal(map.displayDuration, 32_500);
  });

  it("round-trips event times and segment boundaries exactly", () => {
    const times = [5_000, 15_000, 90_000, 95_000, 180_000];
    const map = buildTimeMap(times);
    const boundaries = [
      ...times,
      ...map.gaps.flatMap((gap) => [gap.realStart, gap.realEnd]),
    ];

    for (const t of boundaries) {
      assert.equal(map.toReal(map.toDisplay(t)), t);
    }
    for (const d of map.gaps.flatMap((gap) => [
      gap.displayStart,
      gap.displayEnd,
    ])) {
      assert.equal(map.toDisplay(map.toReal(d)), d);
    }
  });

  it("is monotonic on random sorted input", () => {
    let seed = 9;
    function random() {
      seed = (seed * 1_664_525 + 1_013_904_223) % 2 ** 32;
      return seed / 2 ** 32;
    }

    let t = 0;
    const times = Array.from({ length: 200 }, () => {
      t += Math.floor(random() * 90_000);
      return t;
    });
    const map = buildTimeMap(times);

    let previousDisplay = Number.NEGATIVE_INFINITY;
    for (let index = 0; index < 500; index += 1) {
      const real =
        times[0] + ((times.at(-1) ?? times[0]) - times[0]) * (index / 499);
      const display = map.toDisplay(real);
      assert.ok(
        display >= previousDisplay,
        `expected ${display} >= ${previousDisplay}`,
      );
      previousDisplay = display;
    }

    let previousReal = Number.NEGATIVE_INFINITY;
    for (let index = 0; index < 500; index += 1) {
      const display = map.displayDuration * (index / 499);
      const real = map.toReal(display);
      assert.ok(real >= previousReal, `expected ${real} >= ${previousReal}`);
      previousReal = real;
    }
  });

  it("handles degenerate zero- and one-event inputs", () => {
    const empty = buildTimeMap([]);
    assert.ok(empty.displayDuration >= 1);
    assert.deepEqual(empty.gaps, []);
    assert.equal(empty.toDisplay(123), 0);
    assert.equal(empty.toReal(123), 0);

    const single = buildTimeMap([42_000]);
    assert.ok(single.displayDuration >= 1);
    assert.deepEqual(single.gaps, []);
    assert.equal(single.toDisplay(42_000), 0);
    assert.equal(single.toDisplay(100_000), 0);
    assert.equal(single.toReal(0), 42_000);
    assert.equal(single.toReal(100), 42_000);
  });
});
