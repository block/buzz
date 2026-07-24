import assert from "node:assert/strict";
import test from "node:test";

import {
  bigintRatio,
  buildLocalDayBoundaries,
  deriveUsageIngressTrailing,
  formatEstimatedCostUsd,
  formatTokenCountCompact,
  formatTokenCountExact,
  isPartialField,
  isUnknownField,
  msUntilNextLocalMidnight,
  parseTokenCount,
  sortAgentsByKnownTotal,
  sortModelsByKnownTotal,
  sumKnownBucketTotals,
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

test("buildLocalDayBoundaries returns 8 boundaries for a 7-day window", () => {
  const now = new Date(2026, 5, 15, 14, 30, 0); // June 15, 2026, 14:30 local
  const boundaries = buildLocalDayBoundaries(7, now);
  assert.equal(boundaries.length, 8);
});

test("buildLocalDayBoundaries returns 31 boundaries for a 30-day window", () => {
  const now = new Date(2026, 5, 15, 14, 30, 0);
  const boundaries = buildLocalDayBoundaries(30, now);
  assert.equal(boundaries.length, 31);
});

test("buildLocalDayBoundaries produces strictly increasing boundaries ending at tomorrow's local midnight", () => {
  const now = new Date(2026, 5, 15, 14, 30, 0);
  const boundaries = buildLocalDayBoundaries(7, now);
  for (let i = 1; i < boundaries.length; i++) {
    assert.ok(
      boundaries[i] > boundaries[i - 1],
      `boundary ${i} must exceed boundary ${i - 1}`,
    );
  }
  const tomorrowMidnight = new Date(2026, 5, 16, 0, 0, 0, 0);
  assert.equal(
    boundaries.at(-1),
    Math.floor(tomorrowMidnight.getTime() / 1000),
  );
});

test("buildLocalDayBoundaries is independent of time-of-day within the reference day", () => {
  const morning = buildLocalDayBoundaries(7, new Date(2026, 5, 15, 0, 0, 1));
  const night = buildLocalDayBoundaries(7, new Date(2026, 5, 15, 23, 59, 59));
  assert.deepEqual(morning, night);
});

// ── buildLocalDayBoundaries / msUntilNextLocalMidnight explicit-TZ coverage ──
//
// `process.env.TZ` is read by the JS engine on every `Date` field access
// (not just pinned at process start), so each test below mutates it
// directly and restores the original value in a `finally` — no subprocess
// needed, but evidence (`Date#toString()`) is asserted so a future runtime
// that DOES pin `TZ` at startup fails loudly instead of silently passing
// against the wrong offset.

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

test("buildLocalDayBoundaries stays strictly increasing across an ordinary US spring-forward DST transition", () => {
  withTz("America/New_York", () => {
    const now = new Date(2024, 2, 12, 10, 0, 0); // Mar 12, after Mar 10 spring-forward
    assert.equal(
      now.toString().includes("Daylight"),
      true,
      "sanity: DST active",
    );
    const boundaries = buildLocalDayBoundaries(7, now);
    assert.equal(boundaries.length, 8);
    assertStrictlyIncreasing(boundaries);
  });
});

test("buildLocalDayBoundaries stays strictly increasing across an ordinary US fall-back DST transition", () => {
  withTz("America/New_York", () => {
    const now = new Date(2024, 10, 6, 10, 0, 0); // Nov 6, after Nov 3 fall-back
    assert.equal(
      now.toString().includes("Standard"),
      true,
      "sanity: standard time active",
    );
    const boundaries = buildLocalDayBoundaries(7, now);
    assert.equal(boundaries.length, 8);
    assertStrictlyIncreasing(boundaries);
  });
});

test("buildLocalDayBoundaries stays strictly increasing across Lord Howe Island's 30-minute DST shift", () => {
  withTz("Australia/Lord_Howe", () => {
    const now = new Date(2024, 9, 8, 10, 0, 0); // Oct 8, after Oct 6 spring-forward (+30min)
    const boundaries = buildLocalDayBoundaries(7, now);
    assert.equal(boundaries.length, 8);
    assertStrictlyIncreasing(boundaries);
    // The transition day is a 23.5h day, not the ordinary 24h.
    const deltas = [];
    for (let i = 1; i < boundaries.length; i++) {
      deltas.push(boundaries[i] - boundaries[i - 1]);
    }
    assert.ok(
      deltas.some((d) => d === 23.5 * 3600),
      `expected a 23.5h transition delta, got ${JSON.stringify(deltas)}`,
    );
  });
});

test("buildLocalDayBoundaries stays strictly increasing across Pacific/Apia's skipped 2011-12-30 civil date", () => {
  withTz("Pacific/Apia", () => {
    // Samoa skipped 2011-12-30 entirely moving across the International
    // Date Line; Dec 29 was immediately followed by Dec 31. The UTC
    // offset itself jumped by exactly 24h (UTC-11 -> UTC+13) at that
    // instant, so real elapsed time across the 2-civil-day jump is only
    // 24h, not 48h — but the boundary construction must not collapse the
    // two distinct local midnights (Dec 29, Dec 31) into one duplicate.
    const now = new Date(2011, 11, 31, 12, 0, 0);
    const boundaries = buildLocalDayBoundaries(7, now);
    assert.equal(boundaries.length, 8);
    assertStrictlyIncreasing(boundaries);
    const dec29 = Math.floor(new Date(2011, 11, 29, 0, 0, 0).getTime() / 1000);
    const dec31 = Math.floor(new Date(2011, 11, 31, 0, 0, 0).getTime() / 1000);
    assert.ok(
      boundaries.includes(dec29),
      "expected Dec 29 local midnight as a boundary",
    );
    assert.ok(
      boundaries.includes(dec31),
      "expected Dec 31 local midnight as a boundary",
    );
  });
});

test("buildLocalDayBoundaries produces days+1 boundaries even when a civil date is skipped", () => {
  withTz("Pacific/Apia", () => {
    const boundaries7 = buildLocalDayBoundaries(
      7,
      new Date(2011, 11, 31, 12, 0, 0),
    );
    assert.equal(boundaries7.length, 8);
    const boundaries30 = buildLocalDayBoundaries(
      30,
      new Date(2011, 11, 31, 12, 0, 0),
    );
    assert.equal(boundaries30.length, 31);
    assertStrictlyIncreasing(boundaries30);
  });
});

// ── msUntilNextLocalMidnight ─────────────────────────────────────────────────

test("msUntilNextLocalMidnight returns the exact gap to the next local midnight", () => {
  const now = new Date(2026, 5, 15, 23, 0, 0, 0);
  const ms = msUntilNextLocalMidnight(now);
  assert.equal(ms, 60 * 60 * 1000);
});

test("msUntilNextLocalMidnight is always positive, even called exactly at midnight", () => {
  const now = new Date(2026, 5, 15, 0, 0, 0, 0);
  const ms = msUntilNextLocalMidnight(now);
  assert.equal(ms, 24 * 60 * 60 * 1000);
});

test("msUntilNextLocalMidnight is positive and lands on a real local midnight across DST/date-line TZs", () => {
  const cases = [
    ["America/New_York", new Date(2024, 2, 9, 23, 30, 0)], // eve of spring-forward
    ["Australia/Lord_Howe", new Date(2024, 9, 5, 23, 45, 0)], // eve of 30-min shift
    ["Pacific/Apia", new Date(2011, 11, 29, 23, 0, 0)], // eve of the skipped date
  ];
  for (const [tz, now] of cases) {
    withTz(tz, () => {
      const ms = msUntilNextLocalMidnight(now);
      assert.ok(ms > 0, `${tz}: expected positive ms, got ${ms}`);
      const landed = new Date(now.getTime() + ms);
      assert.equal(
        landed.getHours(),
        0,
        `${tz}: expected to land on local midnight, got ${landed.toString()}`,
      );
      assert.equal(landed.getMinutes(), 0, `${tz}: expected :00 minutes`);
    });
  }
});

// ── parseTokenCount ──────────────────────────────────────────────────────────

test("parseTokenCount parses a plain decimal string to bigint", () => {
  assert.equal(parseTokenCount("12345"), 12345n);
});

test("parseTokenCount preserves u64::MAX precision beyond Number.MAX_SAFE_INTEGER", () => {
  assert.equal(parseTokenCount("18446744073709551615"), 18446744073709551615n);
});

test("parseTokenCount returns null for null input", () => {
  assert.equal(parseTokenCount(null), null);
});

test("parseTokenCount fails closed on malformed wire data instead of throwing", () => {
  for (const malformed of ["", "-1", "1.5", "abc", "1e10", " 1", "1 "]) {
    assert.equal(
      parseTokenCount(malformed),
      null,
      `expected null for ${JSON.stringify(malformed)}`,
    );
  }
});

test("parseTokenCount accepts zero", () => {
  assert.equal(parseTokenCount("0"), 0n);
});

// ── formatTokenCountCompact / formatTokenCountExact ─────────────────────────

test("formatTokenCountCompact abbreviates thousands/millions/billions", () => {
  assert.equal(formatTokenCountCompact(999n), "999");
  assert.equal(formatTokenCountCompact(1_234n), "1.2K");
  assert.equal(formatTokenCountCompact(1_000_000n), "1M");
  assert.equal(formatTokenCountCompact(1_500_000_000n), "1.5B");
});

test("formatTokenCountCompact handles negative magnitudes symmetrically", () => {
  assert.equal(formatTokenCountCompact(-1_234n), "-1.2K");
});

test("formatTokenCountExact renders full grouped digits, never abbreviated", () => {
  assert.equal(formatTokenCountExact(1_234_567n), "1,234,567");
  assert.equal(formatTokenCountExact(0n), "0");
});

// ── formatEstimatedCostUsd ───────────────────────────────────────────────────

test("formatEstimatedCostUsd renders two-decimal USD currency", () => {
  assert.equal(formatEstimatedCostUsd(1.5), "$1.50");
  assert.equal(formatEstimatedCostUsd(0), "$0.00");
});

// ── bigintRatio ──────────────────────────────────────────────────────────────

test("bigintRatio computes a bounded ratio without losing bigint precision on large magnitudes", () => {
  const whole = 18_446_744_073_709_551_614n; // largest even value near u64::MAX
  assert.equal(bigintRatio(whole / 2n, whole), 0.5);
});

test("bigintRatio returns 0 for a zero or negative whole (divide-by-zero guard)", () => {
  assert.equal(bigintRatio(5n, 0n), 0);
  assert.equal(bigintRatio(5n, -10n), 0);
});

test("bigintRatio clamps part to [0, whole]", () => {
  assert.equal(bigintRatio(-5n, 100n), 0);
  assert.equal(bigintRatio(200n, 100n), 1);
});

// ── sortAgentsByKnownTotal / sortModelsByKnownTotal ─────────────────────────

test("sortAgentsByKnownTotal ranks known totals descending", () => {
  const agents = [
    agentUsage("a1", "100"),
    agentUsage("a2", "300"),
    agentUsage("a3", "200"),
  ];
  const sorted = sortAgentsByKnownTotal(agents);
  assert.deepEqual(
    sorted.map((a) => a.agentPubkey),
    ["a2", "a3", "a1"],
  );
});

test("sortAgentsByKnownTotal lists unknown-total agents after all known-total agents, never interleaved", () => {
  const agents = [
    agentUsage("unknown-b", null),
    agentUsage("known", "50"),
    agentUsage("unknown-a", null),
  ];
  const sorted = sortAgentsByKnownTotal(agents);
  assert.equal(sorted[0].agentPubkey, "known");
  // Unknown-total agents tiebreak by normalized pubkey.
  assert.deepEqual(
    sorted.slice(1).map((a) => a.agentPubkey),
    ["unknown-a", "unknown-b"],
  );
});

test("sortAgentsByKnownTotal tiebreaks equal known totals by pubkey", () => {
  const agents = [agentUsage("b", "100"), agentUsage("a", "100")];
  const sorted = sortAgentsByKnownTotal(agents);
  assert.deepEqual(
    sorted.map((a) => a.agentPubkey),
    ["a", "b"],
  );
});

test("sortModelsByKnownTotal sorts null model ('Unknown model') last among ties", () => {
  const models = [
    modelUsage(null, "100"),
    modelUsage("gpt-4", "100"),
    modelUsage("claude", "100"),
  ];
  const sorted = sortModelsByKnownTotal(models);
  assert.deepEqual(
    sorted.map((m) => m.model),
    ["claude", "gpt-4", null],
  );
});

// ── isPartialField / isUnknownField ──────────────────────────────────────────

test("isPartialField is true only for a known value flagged incomplete", () => {
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
});

test("isUnknownField is true only when there is no known value at all", () => {
  assert.equal(isUnknownField(usageField({ value: null })), true);
  assert.equal(isUnknownField(usageField({ value: "0" })), false);
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

test("sumKnownBucketTotals returns knownTotal null and partial false for an all-empty window", () => {
  const result = sumKnownBucketTotals([
    bucket({ reportCount: 0 }),
    bucket({ reportCount: 0 }),
  ]);
  assert.equal(result.knownTotal, null);
  assert.equal(result.partial, false);
});

test("sumKnownBucketTotals sums all known totals when every bucket is fully known", () => {
  const result = sumKnownBucketTotals([
    bucket({
      usage: reportedUsage({ totalTokens: usageField({ value: "100" }) }),
      reportCount: 1,
    }),
    bucket({
      usage: reportedUsage({ totalTokens: usageField({ value: "200" }) }),
      reportCount: 1,
    }),
  ]);
  assert.equal(result.knownTotal, 300n);
  assert.equal(result.partial, false);
});

test("sumKnownBucketTotals marks partial true when any bucket has an incomplete (known lower-bound) total", () => {
  const result = sumKnownBucketTotals([
    bucket({
      usage: reportedUsage({
        totalTokens: usageField({ value: "100", incomplete: true }),
      }),
      reportCount: 1,
    }),
    bucket({
      usage: reportedUsage({ totalTokens: usageField({ value: "200" }) }),
      reportCount: 1,
    }),
  ]);
  assert.equal(result.knownTotal, 300n);
  assert.equal(result.partial, true);
});

test("sumKnownBucketTotals marks partial true when any bucket has reports but null total (activity not fully counted)", () => {
  const result = sumKnownBucketTotals([
    bucket({
      usage: reportedUsage({ totalTokens: usageField({ value: "100" }) }),
      reportCount: 1,
    }),
    bucket({ usage: reportedUsage(), reportCount: 1, hasUnknownUsage: true }),
  ]);
  assert.equal(result.knownTotal, 100n);
  assert.equal(result.partial, true);
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

test("deriveUsageIngressTrailing returns 'Collection off' when collection is disabled", () => {
  const series = baseSeries({ collectionEnabled: false });
  assert.equal(deriveUsageIngressTrailing(series), "Collection off");
});

test("deriveUsageIngressTrailing returns 'No recent data' when collection is on but no agents present", () => {
  const series = baseSeries({ agents: [] });
  assert.equal(deriveUsageIngressTrailing(series), "No recent data");
});

test("deriveUsageIngressTrailing returns compact token count when a known non-partial total is available", () => {
  const series = baseSeries({
    agents: [agentUsage("a", "1500")],
  });
  assert.equal(deriveUsageIngressTrailing(series), "1.5K");
});

test("deriveUsageIngressTrailing appends '· Partial' when the total is a known lower bound", () => {
  const series = baseSeries({
    agents: [
      agentUsage("a", null, {
        usage: reportedUsage({
          totalTokens: usageField({ value: "1500", incomplete: true }),
        }),
      }),
    ],
  });
  assert.equal(deriveUsageIngressTrailing(series), "1.5K · Partial");
});

test("deriveUsageIngressTrailing returns 'Input/output reported' when only input or output is known", () => {
  const series = baseSeries({
    agents: [
      agentUsage("a", null, {
        usage: reportedUsage({
          inputTokens: usageField({ value: "800" }),
          outputTokens: usageField({ value: "200" }),
        }),
      }),
    ],
  });
  assert.equal(deriveUsageIngressTrailing(series), "Input/output reported");
});

test("deriveUsageIngressTrailing appends '· Partial' when only incomplete I/O fields are known", () => {
  const series = baseSeries({
    agents: [
      agentUsage("a", null, {
        usage: reportedUsage({
          inputTokens: usageField({ value: "800", incomplete: true }),
          outputTokens: usageField({ value: "200" }),
        }),
      }),
    ],
  });
  assert.equal(
    deriveUsageIngressTrailing(series),
    "Input/output reported · Partial",
  );
});

test("deriveUsageIngressTrailing returns 'No recent data' when all usage fields are unknown", () => {
  const series = baseSeries({
    agents: [agentUsage("a", null)],
  });
  assert.equal(deriveUsageIngressTrailing(series), "No recent data");
});
