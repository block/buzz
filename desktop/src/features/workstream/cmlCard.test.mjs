import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  formatHeadShort,
  assertNoSensitiveLeak,
  isDisplayableLiveClaim,
  parseWorkstreamCard,
} from "./cmlCard.ts";

// The fixtures were serialized by the real Rust serde implementation in
// crates/buzz-core/src/cml_view.rs — these bytes ARE the wire contract, so
// load them from disk rather than duplicating them inline.
const fixturePath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "cmlCardFixtures.json",
);
const fixtures = JSON.parse(readFileSync(fixturePath, "utf8"));

// Known-correct projections per scenario. Liveness here is the value the Rust
// layer RECOMPUTED at `observed_at` — never a stored presence field, because a
// stored field is frozen at signature time and would show dead workers online.
const EXPECTED = {
  online_live_claim: { liveness: "online", live_claim: true, head_short: "abcdef0" },
  stale_not_live: { liveness: "stale", live_claim: false, head_short: "abcdef0" },
  offline_two_ttl: { liveness: "offline", live_claim: false, head_short: "abcdef0" },
  no_heartbeat_offline: { liveness: "offline", live_claim: false, head_short: "abcdef0" },
  fresh_heartbeat_expired_lease: { liveness: "online", live_claim: false, head_short: "abcdef0" },
  missing_head_sha: { liveness: "online", live_claim: true, head_short: null },
};

const SCENARIOS = Object.keys(EXPECTED);

/** A known-valid card (deep clone) that negative tests can mutate. */
function baseCard() {
  return structuredClone(fixtures.online_live_claim.card);
}

test("fixture file contains exactly the six known scenarios", () => {
  assert.deepEqual([...SCENARIOS].sort(), Object.keys(fixtures).sort());
});

test("parseWorkstreamCard accepts all six fixture scenarios with exact projected values", () => {
  for (const scenario of SCENARIOS) {
    const entry = fixtures[scenario];
    assert.equal(typeof entry.observed_at, "number", `${scenario}: observed_at`);
    const card = parseWorkstreamCard(entry.card);
    const expected = EXPECTED[scenario];
    assert.equal(card.liveness, expected.liveness, `${scenario}: liveness`);
    assert.equal(card.live_claim, expected.live_claim, `${scenario}: live_claim`);
    assert.equal(card.head_short, expected.head_short, `${scenario}: head_short`);
  }
});

test("isDisplayableLiveClaim is true exactly when liveness is online AND live_claim", () => {
  for (const scenario of SCENARIOS) {
    const card = parseWorkstreamCard(fixtures[scenario].card);
    const expected =
      EXPECTED[scenario].liveness === "online" && EXPECTED[scenario].live_claim === true;
    assert.equal(
      isDisplayableLiveClaim(card),
      expected,
      `${scenario}: expected isDisplayableLiveClaim=${expected}`,
    );
  }
});

test("fresh heartbeat with an expired lease is NOT a live claim", () => {
  // The heartbeat is fresh (liveness "online"), but the claim lease has
  // expired. Lease expiry is independent of heartbeat freshness: rendering
  // this as "someone is on it" would show a live claim nobody holds, so the
  // card must not be displayable as a live claim even though the host is up.
  const card = parseWorkstreamCard(fixtures.fresh_heartbeat_expired_lease.card);
  assert.equal(card.liveness, "online");
  assert.equal(card.live_claim, false);
  assert.equal(isDisplayableLiveClaim(card), false);
});

test("missing head_sha yields null head_short and the placeholder, never a fabricated SHA", () => {
  const card = parseWorkstreamCard(fixtures.missing_head_sha.card);
  assert.equal(card.head_short, null);
  const formatted = formatHeadShort(card);
  assert.equal(formatted, "(no head)");
  // The placeholder must be a literal — it must not contain any character of
  // base_short (i.e. no hex scraped from another field to fake a head SHA).
  for (const ch of new Set(card.base_short.split(""))) {
    assert.ok(
      !formatted.includes(ch),
      `placeholder "${formatted}" must not contain base_short character "${ch}"`,
    );
  }
});

test("formatHeadShort returns the real head_short when present", () => {
  const card = parseWorkstreamCard(fixtures.online_live_claim.card);
  assert.equal(formatHeadShort(card), "abcdef0");
});

test("parseWorkstreamCard fails closed on malformed input", () => {
  const cases = [
    {
      label: "missing required field title",
      mutate: (raw) => {
        delete raw.title;
      },
    },
    {
      label: "liveness wrong case ONLINE",
      mutate: (raw) => {
        raw.liveness = "ONLINE";
      },
    },
    {
      label: "liveness unknown value dead",
      mutate: (raw) => {
        raw.liveness = "dead";
      },
    },
    {
      label: "blocker_count wrong primitive type string",
      mutate: (raw) => {
        raw.blocker_count = "0";
      },
    },
    {
      label: "status wrong case Working",
      mutate: (raw) => {
        raw.status = "Working";
      },
    },
  ];

  for (const { label, mutate } of cases) {
    const raw = baseCard();
    mutate(raw);
    assert.throws(
      () => parseWorkstreamCard(raw),
      Error,
      `expected parseWorkstreamCard to throw for: ${label}`,
    );
  }

  // Non-object input must also fail closed, not coerce or default.
  for (const bad of [null, undefined, "card", 42, [], Number.NaN]) {
    assert.throws(() => parseWorkstreamCard(bad), Error);
  }
});

test("parseWorkstreamCard rejects negative blocker_count and review_round", () => {
  for (const field of ["blocker_count", "review_round"]) {
    const raw = baseCard();
    raw[field] = -1;
    assert.throws(() => parseWorkstreamCard(raw), Error, `negative ${field}`);
  }
});

test("assertNoSensitiveLeak throws on absolute paths, raw IPv4, and full SHAs", () => {
  const leaks = [
    {
      label: "absolute filesystem path in worktree_alias",
      mutate: (raw) => {
        raw.worktree_alias = "/Users/someone/src/buzz";
      },
    },
    {
      label: "raw IPv4 host address in host_id",
      mutate: (raw) => {
        raw.host_id = "192.168.1.44";
      },
    },
    {
      label: "full 40-char hex SHA in base_short",
      mutate: (raw) => {
        raw.base_short = "a".repeat(40);
      },
    },
    {
      label: "Windows drive path in branch",
      mutate: (raw) => {
        raw.branch = "C:\\Users\\someone\\src\\buzz";
      },
    },
  ];

  for (const { label, mutate } of leaks) {
    const raw = baseCard();
    mutate(raw);
    const card = parseWorkstreamCard(raw);
    assert.throws(
      () => assertNoSensitiveLeak(card),
      Error,
      `expected assertNoSensitiveLeak to throw for: ${label}`,
    );
  }
});

test("assertNoSensitiveLeak accepts all six real fixture scenarios", () => {
  for (const scenario of SCENARIOS) {
    const card = parseWorkstreamCard(fixtures[scenario].card);
    assert.doesNotThrow(() => assertNoSensitiveLeak(card), scenario);
  }
});
