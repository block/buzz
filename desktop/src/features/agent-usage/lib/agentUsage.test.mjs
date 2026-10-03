import assert from "node:assert/strict";
import test from "node:test";

import {
  bigintRatio,
  buildCustomDayBoundaries,
  buildLocalDayBoundaries,
  buildRangeBoundaries,
  countRangeDays,
  describeRange,
  deriveDisplayTotal,
  deriveUsageIngressTrailing,
  formatCoverageDate,
  formatEstimatedCostUsd,
  formatLocalDate,
  formatModelCacheBreakdown,
  formatTokenCountCompact,
  formatTokenCountExact,
  hasKnownCacheData,
  isPartialField,
  isUnknownField,
  MAX_RANGE_DAYS,
  msUntilNextLocalMidnight,
  parseLocalDate,
  parseTokenCount,
  sortAgentsByDisplayTotal,
  sortModelsByDisplayTotal,
  sumKnownBucketTotals,
  validateCustomRange,
} from "./agentUsage.ts";

// ── Fixtures ─────────────────────────────────────────────────────────────────

function usageField(overrides = {}) {
  return { value: null, incomplete: false, ...overrides };
}

function reportedUsage(overrides = {}) {
  return {
    inputTokens: usageField(),
    outputTokens: usageField(),
    totalTokens: usageField(),
    estimatedCostUsd: usageField(),
    cacheReadTokens: usageField(),
    cacheWriteTokens: usageField(),
    freshInputTokens: usageField(),
    ...overrides,
  };
}

function agentUsage(pubkey, totalTokensValue, overrides = {}) {
  return {
    agentPubkey: pubkey,
    usage: reportedUsage({
      totalTokens: usageField({ value: totalTokensValue }),
    }),
    buckets: [],
    models: [],
    reportCount: 0,
    hasUnknownUsage: false,
    ...overrides,
  };
}

function modelUsage(model, totalTokensValue, overrides = {}) {
  return {
    harness: null,
    model,
    usage: reportedUsage({
      totalTokens: usageField({ value: totalTokensValue }),
    }),
    reportCount: 0,
    hasUnknownUsage: false,
    ...overrides,
  };
}

// ── buildLocalDayBoundaries ──────────────────────────────────────────────────

test("buildLocalDayBoundaries yields days+1 strictly increasing boundaries ending at tomorrow's local midnight", () => {
  const now = new Date(2026, 5, 15, 14, 30, 0); // June 15, 2026, 14:30 local
  for (const days of [7, 30]) {
    const boundaries = buildLocalDayBoundaries(days, now);
    assert.equal(boundaries.length, days + 1);
    assertStrictlyIncreasing(boundaries);
    assert.equal(
      boundaries.at(-1),
      Math.floor(new Date(2026, 5, 16, 0, 0, 0, 0).getTime() / 1000),
    );
  }
  // Result must be independent of time-of-day within the reference date.
  assert.deepEqual(
    buildLocalDayBoundaries(7, new Date(2026, 5, 15, 0, 0, 1)),
    buildLocalDayBoundaries(7, new Date(2026, 5, 15, 23, 59, 59)),
  );
});

// ── TZ helpers ───────────────────────────────────────────────────────────────
// `process.env.TZ` is read on every `Date` field access (not pinned at start),
// so mutating it inline is safe; `finally` restores the original value.

function withTz(tz, fn) {
  const original = process.env.TZ;
  process.env.TZ = tz;
  try {
    return fn();
  } finally {
    if (original === undefined) delete process.env.TZ;
    else process.env.TZ = original;
  }
}

function assertStrictlyIncreasing(boundaries) {
  for (let i = 1; i < boundaries.length; i++) {
    assert.ok(
      boundaries[i] > boundaries[i - 1],
      `boundary ${i} (${boundaries[i]}) must exceed boundary ${i - 1} (${boundaries[i - 1]})`,
    );
  }
}

test("buildLocalDayBoundaries stays strictly increasing across DST transitions", () => {
  const cases = [
    // Mar 12, after the Mar 10 spring-forward (UTC-4).
    ["America/New_York", new Date(2024, 2, 12, 10, 0, 0), 240],
    // Nov 6, after the Nov 3 fall-back (UTC-5).
    ["America/New_York", new Date(2024, 10, 6, 10, 0, 0), 300],
  ];
  for (const [tz, now, expectedOffsetMinutes] of cases) {
    withTz(tz, () => {
      assert.equal(
        now.getTimezoneOffset(),
        expectedOffsetMinutes,
        `sanity: unexpected UTC offset for ${now.toString()}`,
      );
      const boundaries = buildLocalDayBoundaries(7, now);
      assert.equal(boundaries.length, 8);
      assertStrictlyIncreasing(boundaries);
    });
  }
});

test("buildLocalDayBoundaries emits days+1 distinct midnights across Pacific/Apia's skipped 2011-12-30 civil date", () => {
  withTz("Pacific/Apia", () => {
    // Samoa skipped 2011-12-30 when it crossed the Date Line (UTC-11→UTC+13);
    // Dec 29 was immediately followed by Dec 31. Boundaries must not collapse
    // the two distinct local midnights into one duplicate.
    const now = new Date(2011, 11, 31, 12, 0, 0);
    const boundaries = buildLocalDayBoundaries(7, now);
    assert.equal(boundaries.length, 8);
    assertStrictlyIncreasing(boundaries);
    for (const [label, date] of [
      ["Dec 29", new Date(2011, 11, 29, 0, 0, 0)],
      ["Dec 31", new Date(2011, 11, 31, 0, 0, 0)],
    ]) {
      assert.ok(
        boundaries.includes(Math.floor(date.getTime() / 1000)),
        `expected ${label} local midnight as a boundary`,
      );
    }

    const boundaries30 = buildLocalDayBoundaries(30, now);
    assert.equal(boundaries30.length, 31);
    assertStrictlyIncreasing(boundaries30);
  });
});

// ── msUntilNextLocalMidnight ─────────────────────────────────────────────────

test("msUntilNextLocalMidnight returns exact gap to the next local midnight, always positive", () => {
  // Basic: 23:00 → 1h gap; exactly at midnight → 24h gap.
  assert.equal(
    msUntilNextLocalMidnight(new Date(2026, 5, 15, 23, 0, 0, 0)),
    60 * 60 * 1000,
  );
  assert.equal(
    msUntilNextLocalMidnight(new Date(2026, 5, 15, 0, 0, 0, 0)),
    24 * 60 * 60 * 1000,
  );
  // DST/date-line TZs: must be positive and land exactly on local midnight.
  for (const [tz, now] of [
    ["America/New_York", new Date(2024, 2, 9, 23, 30, 0)], // eve of spring-forward
    ["Australia/Lord_Howe", new Date(2024, 9, 5, 23, 45, 0)], // eve of 30-min shift
    ["Pacific/Apia", new Date(2011, 11, 29, 23, 0, 0)], // eve of the skipped date
  ]) {
    withTz(tz, () => {
      const ms = msUntilNextLocalMidnight(now);
      assert.ok(ms > 0, `${tz}: expected positive ms, got ${ms}`);
      const landed = new Date(now.getTime() + ms);
      assert.equal(landed.getHours(), 0, `${tz}: expected local midnight`);
      assert.equal(landed.getMinutes(), 0, `${tz}: expected :00 minutes`);
    });
  }
});

// ── parseTokenCount ──────────────────────────────────────────────────────────

test("parseTokenCount parses decimal strings to bigint, preserving u64 precision", () => {
  for (const [wire, expected] of [
    ["12345", 12345n],
    ["0", 0n],
    // Past Number.MAX_SAFE_INTEGER — a Number round-trip would lose digits.
    ["18446744073709551615", 18446744073709551615n],
  ]) {
    assert.equal(parseTokenCount(wire), expected, wire);
  }
});

test("parseTokenCount fails closed on null or malformed wire data instead of throwing", () => {
  for (const malformed of [null, "", "-1", "1.5", "abc", "1e10", " 1", "1 "]) {
    assert.equal(
      parseTokenCount(malformed),
      null,
      `expected null for ${JSON.stringify(malformed)}`,
    );
  }
});

// ── Formatters ──────────────────────────────────────────────────────────────

test("formatTokenCountCompact abbreviates by magnitude and keeps the sign", () => {
  for (const [count, expected] of [
    [999n, "999"],
    [1_234n, "1.2K"],
    [1_000_000n, "1M"],
    [1_500_000_000n, "1.5B"],
    [-1_234n, "-1.2K"],
  ]) {
    assert.equal(formatTokenCountCompact(count), expected);
  }
});

test("formatTokenCountExact renders full grouped digits, never abbreviated", () => {
  assert.equal(formatTokenCountExact(1_234_567n), "1,234,567");
  assert.equal(formatTokenCountExact(0n), "0");
});

test("formatEstimatedCostUsd renders two-decimal USD currency", () => {
  assert.equal(formatEstimatedCostUsd(1.5), "$1.50");
  assert.equal(formatEstimatedCostUsd(0), "$0.00");
});

test("formatCoverageDate renders unknown for null and omits the year for real timestamps", () => {
  assert.equal(formatCoverageDate(null), "unknown");
  const unixSeconds = 1_737_849_600; // 2025-01-26T00:00:00Z
  const localDate = new Date(unixSeconds * 1000);
  const formatted = formatCoverageDate(unixSeconds);
  assert.match(formatted, new RegExp(`\\b${localDate.getDate()}\\b`));
  assert.doesNotMatch(formatted, new RegExp(`${localDate.getFullYear()}`));
});

// ── bigintRatio ──────────────────────────────────────────────────────────────

test("bigintRatio computes bounded ratios without losing bigint precision", () => {
  const whole = 18_446_744_073_709_551_614n; // largest even value near u64::MAX
  assert.equal(bigintRatio(whole / 2n, whole), 0.5);
  for (const [part, w, expected] of [
    [5n, 0n, 0],
    [5n, -10n, 0],
    [-5n, 100n, 0],
    [200n, 100n, 1],
  ]) {
    assert.equal(bigintRatio(part, w), expected, `${part}/${w}`);
  }
});

// ── deriveDisplayTotal ────────────────────────────────────────────────────────

test("deriveDisplayTotal classifies each usage shape, failing closed on a half-known split", () => {
  const cases = [
    [
      "a reported total is exact",
      { inputTokens: "800", outputTokens: "200", totalTokens: "1100" },
      { kind: "exact", value: 1100n, partial: false },
    ],
    [
      "an exact total flagged incomplete carries partial",
      { totalTokens: ["900", true] },
      { kind: "exact", value: 900n, partial: true },
    ],
    [
      "a null total with both i/o known is approximate",
      { inputTokens: "800", outputTokens: "200" },
      { kind: "approximate", value: 1000n, partial: false },
    ],
    [
      "an incomplete i/o field makes the approximation partial",
      { inputTokens: ["800", true], outputTokens: "200" },
      { kind: "approximate", value: 1000n, partial: true },
    ],
    // Fail-closed: half of a split is never enough to publish a total.
    [
      "input alone is unknown",
      { inputTokens: "500" },
      { kind: "unknown", value: null, partial: false },
    ],
    [
      "output alone is unknown",
      { outputTokens: "300" },
      { kind: "unknown", value: null, partial: false },
    ],
    [
      "no reported field at all is unknown",
      {},
      { kind: "unknown", value: null, partial: false },
    ],
  ];

  for (const [label, fields, expected] of cases) {
    const usage = reportedUsage(
      Object.fromEntries(
        Object.entries(fields).map(([name, field]) => {
          const [value, incomplete = false] = Array.isArray(field)
            ? field
            : [field];
          return [name, usageField({ value, incomplete })];
        }),
      ),
    );
    const dt = deriveDisplayTotal(usage);
    assert.deepEqual(
      { kind: dt.kind, value: dt.value, partial: dt.partial },
      expected,
      label,
    );
  }
});

// ── sortAgentsByDisplayTotal / sortModelsByDisplayTotal ─────────────────────

/** An agent whose total is null but whose i/o sum is known — approximate tier. */
function approxAgent(pubkey, input, output) {
  return agentUsage(pubkey, null, {
    usage: reportedUsage({
      inputTokens: usageField({ value: input }),
      outputTokens: usageField({ value: output }),
    }),
  });
}

test("sortAgentsByDisplayTotal ranks by tier first, then value descending, then pubkey", () => {
  const cases = [
    [
      "known exact totals sort descending",
      [
        agentUsage("a1", "100"),
        agentUsage("a2", "300"),
        agentUsage("a3", "200"),
      ],
      ["a2", "a3", "a1"],
    ],
    [
      // exact(50) < approximate(18000) numerically, but the exact tier wins.
      "an exact tier outranks a larger approximate total",
      [approxAgent("approx", "9000", "9000"), agentUsage("exact", "50")],
      ["exact", "approx"],
    ],
    [
      "an approximate tier outranks an unknown total",
      [agentUsage("unknown", null), approxAgent("approx", "100", "50")],
      ["approx", "unknown"],
    ],
    [
      "a mixed population lands in exact → approximate → unknown order",
      [
        agentUsage("u1", null),
        agentUsage("e1", "100"),
        approxAgent("a1", "400", "100"),
        agentUsage("u2", null),
        agentUsage("e2", "300"),
        approxAgent("a2", "150", "50"),
      ],
      ["e2", "e1", "a1", "a2", "u1", "u2"],
    ],
    [
      "equal totals and unknown totals alike tiebreak by pubkey",
      [agentUsage("b", "100"), agentUsage("a", "100"), agentUsage("c", null)],
      ["a", "b", "c"],
    ],
  ];

  for (const [label, agents, expected] of cases) {
    assert.deepEqual(
      sortAgentsByDisplayTotal(agents).map((a) => a.agentPubkey),
      expected,
      label,
    );
  }
});

test("sortModelsByDisplayTotal ranks by tier, then tiebreaks harness before model with nulls last", () => {
  const equalTotals = [
    modelUsage(null, "100"),
    modelUsage("gpt-4", "100"),
    modelUsage("claude", "100"),
  ];
  assert.deepEqual(
    sortModelsByDisplayTotal(equalTotals).map((m) => m.model),
    ["claude", "gpt-4", null],
    "a null model ('Unknown model') sorts last among ties",
  );

  const sameModelManyHarnesses = [
    modelUsage("m", "100", { harness: "z-harness" }),
    modelUsage("m", "100", { harness: "a-harness" }),
    modelUsage("m", "100", { harness: null }),
  ];
  assert.deepEqual(
    sortModelsByDisplayTotal(sameModelManyHarnesses).map((m) => m.harness),
    ["a-harness", "z-harness", null],
    "the same model via several harnesses stays distinct, in harness order",
  );

  const mixedTiers = [
    modelUsage("big-approx", null, {
      usage: reportedUsage({
        inputTokens: usageField({ value: "9999" }),
        outputTokens: usageField({ value: "9999" }),
      }),
    }),
    modelUsage("small-model", "10"),
  ];
  assert.deepEqual(
    sortModelsByDisplayTotal(mixedTiers).map((m) => m.model),
    ["small-model", "big-approx"],
    "the exact tier outranks a larger approximate total",
  );
});

// ── isPartialField / isUnknownField ──────────────────────────────────────────

test("isPartialField and isUnknownField classify usage fields correctly", () => {
  assert.equal(
    isPartialField(usageField({ value: "10", incomplete: true })),
    true,
  );
  assert.equal(
    isPartialField(usageField({ value: "10", incomplete: false })),
    false,
  );
  assert.equal(
    isPartialField(usageField({ value: null, incomplete: true })),
    false,
  );
  assert.equal(isUnknownField(usageField({ value: null })), true);
  assert.equal(isUnknownField(usageField({ value: "0" })), false);
});

// ── formatModelCacheBreakdown / hasKnownCacheData ────────────────────────────

test("hasKnownCacheData is true iff any cache subset carries a known value", () => {
  assert.equal(hasKnownCacheData(reportedUsage()), false, "all absent → false");
  for (const field of [
    "cacheReadTokens",
    "cacheWriteTokens",
    "freshInputTokens",
  ]) {
    assert.equal(
      hasKnownCacheData(reportedUsage({ [field]: usageField({ value: "0" }) })),
      true,
      `a known ${field} (even zero) → true`,
    );
  }
  // A field that is incomplete but has no value is still unknown, not known.
  assert.equal(
    hasKnownCacheData(
      reportedUsage({
        cacheReadTokens: usageField({ value: null, incomplete: true }),
      }),
    ),
    false,
    "incomplete with null value is unknown, not known",
  );
});

test("formatModelCacheBreakdown omits absent subsets, never renders them as zero, and marks a known lower bound Partial", () => {
  const build = (overrides) =>
    modelUsage("m", null, { usage: reportedUsage(overrides) });

  // No cache data at all → null, so the caller omits the line entirely.
  assert.equal(formatModelCacheBreakdown(build({})), null);

  // Each known subset appears compact-formatted; absent ones are omitted
  // rather than shown as "0".
  assert.equal(
    formatModelCacheBreakdown(
      build({
        cacheReadTokens: usageField({ value: "1500" }),
        cacheWriteTokens: usageField({ value: "300" }),
        freshInputTokens: usageField({ value: "1200000" }),
      }),
    ),
    "Cache read 1.5K · Cache write 300 · Fresh 1.2M",
  );

  // Only cache-read known — the other two are unknown and omitted, not zero.
  assert.equal(
    formatModelCacheBreakdown(
      build({ cacheReadTokens: usageField({ value: "800" }) }),
    ),
    "Cache read 800",
  );

  // An incomplete known field appends a single trailing Partial marker.
  assert.equal(
    formatModelCacheBreakdown(
      build({
        cacheReadTokens: usageField({ value: "800", incomplete: true }),
        cacheWriteTokens: usageField({ value: "200" }),
      }),
    ),
    "Cache read 800 · Cache write 200 · Partial",
  );

  // An incomplete field with NO value carries no known lower bound: it is
  // omitted and does not trigger Partial on its own.
  assert.equal(
    formatModelCacheBreakdown(
      build({
        cacheReadTokens: usageField({ value: "800" }),
        cacheWriteTokens: usageField({ value: null, incomplete: true }),
      }),
    ),
    "Cache read 800",
  );
});

// ── sumKnownBucketTotals ──────────────────────────────────────────────────────

function bucket(overrides = {}) {
  return {
    start: 1_700_000_000,
    end: 1_700_086_400,
    usage: reportedUsage(),
    reportCount: 0,
    hasUnknownUsage: false,
    ...overrides,
  };
}

/** A report-bearing bucket whose total is exactly `value`. */
function exactBucket(value, incomplete = false) {
  return bucket({
    usage: reportedUsage({ totalTokens: usageField({ value, incomplete }) }),
    reportCount: 1,
  });
}

/** A report-bearing bucket with no total but a known i/o split. */
function approxBucket(input, output, incomplete = false) {
  return bucket({
    usage: reportedUsage({
      inputTokens: usageField({ value: input, incomplete }),
      outputTokens: usageField({ value: output }),
    }),
    reportCount: 1,
  });
}

/** A report-bearing bucket with nothing countable at all. */
function unknownBucket() {
  return bucket({
    usage: reportedUsage(),
    reportCount: 1,
    hasUnknownUsage: true,
  });
}

test("sumKnownBucketTotals aggregates buckets without erasing or fabricating a subtotal", () => {
  const cases = [
    [
      "a window of empty buckets is unknown, not zero",
      [bucket({ reportCount: 0 }), bucket({ reportCount: 0 })],
      { kind: "unknown", value: null, partial: false },
    ],
    [
      "fully-known totals sum exactly",
      [exactBucket("100"), exactBucket("200")],
      { kind: "exact", value: 300n, partial: false },
    ],
    [
      "an incomplete (known lower-bound) total marks the sum partial",
      [exactBucket("100", true), exactBucket("200")],
      { kind: "exact", value: 300n, partial: true },
    ],
    [
      "an unknown sibling preserves the known exact subtotal as partial",
      [exactBucket("100"), unknownBucket()],
      { kind: "exact", value: 100n, partial: true },
    ],
    [
      "all-null totals with known i/o sum to an approximation",
      [approxBucket("800", "200"), approxBucket("400", "100")],
      { kind: "approximate", value: 1500n, partial: false },
    ],
    [
      "an incomplete i/o field marks the approximation partial",
      [approxBucket("800", "200", true), approxBucket("400", "100")],
      { kind: "approximate", value: 1500n, partial: true },
    ],
    [
      "mixed exact and approximate buckets aggregate as approximate",
      [exactBucket("1000"), approxBucket("300", "200")],
      { kind: "approximate", value: 1500n, partial: false },
    ],
    [
      "an unknown sibling preserves the known approximate subtotal as partial",
      [approxBucket("400", "100"), unknownBucket()],
      { kind: "approximate", value: 500n, partial: true },
    ],
    [
      "a lone report-bearing bucket with no countable field is unknown",
      [unknownBucket()],
      { kind: "unknown", value: null, partial: false },
    ],
    [
      "a window where no report-bearing bucket has a display value is unknown",
      [unknownBucket(), unknownBucket()],
      { kind: "unknown", value: null, partial: false },
    ],
  ];

  for (const [label, buckets, expected] of cases) {
    const result = sumKnownBucketTotals(buckets);
    assert.deepEqual(
      { kind: result.kind, value: result.value, partial: result.partial },
      expected,
      label,
    );
  }
});

// ── deriveUsageIngressTrailing ────────────────────────────────────────────────

function baseSeries(overrides = {}) {
  return {
    collectionEnabled: true,
    buckets: [],
    agents: [],
    coverage: {
      firstArchivedAt: null,
      firstReportedAt: null,
      hasUnknownUsage: false,
      invalidReportCount: 0,
      lastArchivedAt: null,
      lastReportedAt: null,
      reportCount: 0,
    },
    hasArchivedEvidence: null,
    ...overrides,
  };
}

test("deriveUsageIngressTrailing summarizes the series, marking a known lower bound Partial", () => {
  const io = (input, output, incomplete = false) =>
    reportedUsage({
      inputTokens: usageField({ value: input, incomplete }),
      outputTokens: usageField({ value: output }),
    });

  const cases = [
    ["collection disabled", { collectionEnabled: false }, "Collection off"],
    ["collection on with no agents", { agents: [] }, "No recent data"],
    [
      "a known non-partial total",
      { agents: [agentUsage("a", "1500")] },
      "1.5K",
    ],
    [
      "a total that is only a known lower bound",
      {
        agents: [
          agentUsage("a", null, {
            usage: reportedUsage({
              totalTokens: usageField({ value: "1500", incomplete: true }),
            }),
          }),
        ],
      },
      "1.5K · Partial",
    ],
    [
      "i/o known but no total",
      { agents: [agentUsage("a", null, { usage: io("800", "200") })] },
      "Input/output reported",
    ],
    [
      "i/o known but one field incomplete",
      { agents: [agentUsage("a", null, { usage: io("800", "200", true) })] },
      "Input/output reported · Partial",
    ],
    // Fail-closed: an agent present with nothing countable is not "0".
    [
      "every usage field unknown",
      { agents: [agentUsage("a", null)] },
      "No recent data",
    ],
  ];

  for (const [label, overrides, expected] of cases) {
    assert.equal(
      deriveUsageIngressTrailing(baseSeries(overrides)),
      expected,
      label,
    );
  }
});

// ── Custom-range parsing, validation, and boundary construction ──────────────

test("parseLocalDate resolves a YYYY-MM-DD string to local midnight, not UTC midnight", () => {
  withTz("America/New_York", () => {
    const parsed = parseLocalDate("2026-03-15");
    assert.notEqual(parsed, null);
    // `new Date("2026-03-15")` is UTC midnight = Mar 14 20:00 in New York.
    assert.equal(parsed.getFullYear(), 2026);
    assert.equal(parsed.getMonth(), 2);
    assert.equal(parsed.getDate(), 15);
    assert.equal(parsed.getHours(), 0);
    assert.notEqual(parsed.getTime(), new Date("2026-03-15").getTime());
  });
});

test("parseLocalDate rejects malformed input and dates that do not exist", () => {
  for (const value of [
    "",
    "x",
    "2026-3-15",
    "15/03/2026",
    "2026-03-15T00:00",
    // `new Date(2026, 1, 30)` silently normalizes to Mar 2 — querying the
    // wrong civil day. The guard must reject these outright.
    "2026-02-30",
    "2026-13-01",
    "2026-00-10",
    "2026-02-29", // not a leap year
  ]) {
    assert.equal(parseLocalDate(value), null, `expected null for ${value}`);
  }
  assert.notEqual(
    parseLocalDate("2024-02-29"),
    null,
    "a leap day in a leap year is valid",
  );
});

test("formatLocalDate round-trips through parseLocalDate", () => {
  withTz("America/New_York", () => {
    for (const value of ["2026-01-01", "2026-03-08", "2026-12-31"]) {
      assert.equal(formatLocalDate(parseLocalDate(value)), value);
    }
  });
});

test("countRangeDays counts civil days and returns null for inverted or malformed ranges", () => {
  assert.equal(countRangeDays("2026-05-04", "2026-05-04"), 1, "inclusive");
  withTz("America/New_York", () => {
    // Mar 8 2026 is spring-forward: 3 civil days, not 3 × 24h.
    assert.equal(countRangeDays("2026-03-07", "2026-03-09"), 3);
  });
  assert.ok(
    countRangeDays("1900-01-01", "2100-01-01") > MAX_RANGE_DAYS,
    "an absurd range reports over-cap rather than walking it",
  );
  assert.equal(countRangeDays("2026-05-10", "2026-05-01"), null);
  assert.equal(countRangeDays("nope", "2026-05-01"), null);
  assert.equal(countRangeDays("2026-05-01", "2026-02-30"), null);
});

test("validateCustomRange accepts the cap exactly and rejects over-cap, inverted, and malformed ranges", () => {
  // 2024 is a leap year: Jan 1 – Dec 31 inclusive is exactly 366 civil days.
  assert.deepEqual(validateCustomRange("2024-01-01", "2024-12-31"), {
    ok: true,
    days: MAX_RANGE_DAYS,
  });
  const overCap = validateCustomRange("2024-01-01", "2025-01-01");
  assert.equal(overCap.ok, false);
  assert.match(overCap.message, /366 days or fewer/);
  const inverted = validateCustomRange("2026-05-10", "2026-05-01");
  assert.equal(inverted.ok, false);
  assert.match(inverted.message, /on or before/);
  for (const [start, end] of [
    ["", "2026-05-01"],
    ["2026-05-01", ""],
    ["2026-02-30", "2026-05-01"],
  ]) {
    const r = validateCustomRange(start, end);
    assert.equal(r.ok, false);
    assert.match(r.message, /start and an end date/);
  }
});

test("buildCustomDayBoundaries returns days+1 strictly increasing boundaries closing the final day", () => {
  withTz("America/New_York", () => {
    // Three-day range: 4 boundaries.
    const boundaries = buildCustomDayBoundaries("2026-05-01", "2026-05-03");
    assert.equal(boundaries.length, 4);
    assertStrictlyIncreasing(boundaries);
    assert.equal(
      boundaries[0],
      Math.floor(new Date(2026, 4, 1).getTime() / 1_000),
    );
    assert.equal(
      boundaries.at(-1),
      Math.floor(new Date(2026, 4, 4).getTime() / 1_000),
      "final boundary opens the day after the requested end date",
    );
    // Single-day range: exactly 2 boundaries.
    const single = buildCustomDayBoundaries("2026-05-04", "2026-05-04");
    assert.equal(single.length, 2);
    assertStrictlyIncreasing(single);
  });
});

test("buildCustomDayBoundaries stays strictly increasing across a spring-forward DST transition", () => {
  withTz("America/New_York", () => {
    const boundaries = buildCustomDayBoundaries("2026-03-06", "2026-03-10");
    assert.equal(boundaries.length, 6);
    assertStrictlyIncreasing(boundaries);
  });
});

test("buildCustomDayBoundaries emits no duplicate boundary across a skipped civil date", () => {
  withTz("Pacific/Apia", () => {
    // 2011-12-30 does not exist in Apia (date-line move): distinct midnights required.
    const boundaries = buildCustomDayBoundaries("2011-12-28", "2011-12-31");
    assertStrictlyIncreasing(boundaries);
    assert.equal(new Set(boundaries).size, boundaries.length);
  });
});

test("buildCustomDayBoundaries produces the maximum boundary count at the cap", () => {
  withTz("America/New_York", () => {
    const boundaries = buildCustomDayBoundaries("2024-01-01", "2024-12-31");
    assert.equal(boundaries.length, MAX_RANGE_DAYS + 1);
    assertStrictlyIncreasing(boundaries);
  });
});

test("buildCustomDayBoundaries returns no boundaries for a range the picker rejects", () => {
  assert.deepEqual(buildCustomDayBoundaries("2026-05-10", "2026-05-01"), []);
  assert.deepEqual(buildCustomDayBoundaries("2024-01-01", "2025-01-01"), []);
  assert.deepEqual(buildCustomDayBoundaries("", ""), []);
});

test("buildRangeBoundaries delegates to buildLocalDayBoundaries for presets and buildCustomDayBoundaries for custom", () => {
  const now = new Date(2026, 5, 15, 12, 0, 0);
  for (const days of [1, 7, 30]) {
    assert.deepEqual(
      buildRangeBoundaries({ kind: "preset", days }, now),
      buildLocalDayBoundaries(days, now),
      `preset ${days}d must not diverge from the shared day walk`,
    );
  }
  // 1-day preset: exactly 2 boundaries, today's and tomorrow's local midnight.
  const oneDayBounds = buildRangeBoundaries({ kind: "preset", days: 1 }, now);
  assert.equal(oneDayBounds.length, 2);
  assert.equal(
    oneDayBounds[0],
    Math.floor(new Date(2026, 5, 15).getTime() / 1_000),
  );
  assert.equal(
    oneDayBounds[1],
    Math.floor(new Date(2026, 5, 16).getTime() / 1_000),
  );
  // Custom range delegates to buildCustomDayBoundaries.
  assert.deepEqual(
    buildRangeBoundaries({
      kind: "custom",
      startDate: "2026-05-01",
      endDate: "2026-05-03",
    }),
    buildCustomDayBoundaries("2026-05-01", "2026-05-03"),
  );
});

test("describeRange renders preset copy and custom date spans", () => {
  assert.equal(describeRange({ kind: "preset", days: 1 }), "the last day");
  assert.equal(describeRange({ kind: "preset", days: 7 }), "the last 7 days");
  assert.equal(describeRange({ kind: "preset", days: 30 }), "the last 30 days");
  const custom = describeRange({
    kind: "custom",
    startDate: "2026-05-01",
    endDate: "2026-05-03",
  });
  assert.match(custom, /2026/);
  assert.match(custom, /–/);
});
