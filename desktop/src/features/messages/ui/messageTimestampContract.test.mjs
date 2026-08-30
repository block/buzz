import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

/**
 * MessageTimestamp renders through Radix tooltip primitives, which need a DOM to
 * mount. These assertions read the source instead, because the thing worth
 * pinning is which formatter each mode reaches for — the labels themselves are
 * covered by `shared/lib/datetime.test.mjs`.
 */
const source = readFileSync(
  new URL("./MessageTimestamp.tsx", import.meta.url),
  "utf8",
).replace(/\s+/g, " ");

test("a message header shows the relative label, including the time", () => {
  // Regression guard: this used to render a bare clock time, so a message from
  // last week read "9:05 AM" once its day divider scrolled out of view.
  assert.match(
    source,
    /: formatItemTimestamp\(createdAt, \{ withTime: true \}\)/,
  );
});

test("the continuation gutter stays clock-only", () => {
  // 36px of width (w-9). A relative label would not fit.
  assert.match(
    source,
    /hideDayPeriod \? formatTimeWithoutDayPeriod\(formatTime\(createdAt\)\)/,
  );
});

test("both labels derive from createdAt, not a pre-formatted prop", () => {
  // A captured string would keep saying "Today" after midnight.
  assert.doesNotMatch(source, /time: string/);
  assert.doesNotMatch(source, /\btime\b(?!\w)[^;]*?=\s*\{/);
});

test("the tooltip still carries the unabbreviated timestamp", () => {
  assert.match(source, /formatFullDateTime\(createdAt\)/);
});
