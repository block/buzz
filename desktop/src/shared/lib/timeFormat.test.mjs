import assert from "node:assert/strict";
import test from "node:test";

import {
  createClockFormatter,
  getTimeFormatPreference,
  resolvesToHour12,
  setTimeFormatPreference,
  withClockOptions,
} from "./timeFormat.ts";

// A fixed local wall-clock instant: 2026-04-02, 14:34 local time.
const AFTERNOON = new Date(2026, 3, 2, 14, 34);
// Local midnight — the case that separates an h23 cycle ("00:07") from h24
// ("24:07"), which some locale defaults would otherwise pick.
const MIDNIGHT = new Date(2026, 3, 2, 0, 7);

function resetPreference() {
  setTimeFormatPreference("12-hour");
}

// Runs first on purpose: nothing has called the setter yet, so this observes
// the value the module resolved at import time (no localStorage under node).
test("defaults to the 12-hour clock the app shipped with", () => {
  assert.equal(getTimeFormatPreference(), "12-hour");
});

test("setTimeFormatPreference round-trips through the module", () => {
  resetPreference();

  setTimeFormatPreference("24-hour");
  assert.equal(getTimeFormatPreference(), "24-hour");

  setTimeFormatPreference("system");
  assert.equal(getTimeFormatPreference(), "system");

  resetPreference();
  assert.equal(getTimeFormatPreference(), "12-hour");
});

test("resolvesToHour12 answers literally for explicit clocks", () => {
  assert.equal(resolvesToHour12("12-hour"), true);
  assert.equal(resolvesToHour12("24-hour"), false);
});

test("resolvesToHour12 defaults to the active preference", () => {
  resetPreference();
  assert.equal(resolvesToHour12(), true);

  setTimeFormatPreference("24-hour");
  assert.equal(resolvesToHour12(), false);

  resetPreference();
});

test("resolvesToHour12 resolves `system` to a boolean, not undefined", () => {
  // The host locale decides which one, so only assert it is a real answer —
  // callers pass it straight into Intl options.
  assert.equal(typeof resolvesToHour12("system"), "boolean");
});

test("withClockOptions pins h23 for 24-hour and clears any hardcoded hour12", () => {
  const options = withClockOptions(
    { hour: "numeric", minute: "2-digit", hour12: true },
    false,
  );

  assert.equal(options.hour12, undefined);
  assert.equal(options.hourCycle, "h23");
  assert.equal(options.hour, "numeric");
  assert.equal(options.minute, "2-digit");
});

test("withClockOptions asks for hour12 without an hourCycle for 12-hour", () => {
  const options = withClockOptions({ hour: "numeric" }, true);

  assert.equal(options.hour12, true);
  assert.equal(options.hourCycle, undefined);
});

test("withClockOptions stays legal alongside dateStyle/timeStyle", () => {
  // Intl throws when dateStyle/timeStyle is combined with component options
  // like `hour`, but permits hour12/hourCycle — the commit detail panel relies
  // on that, so assert both directions actually construct.
  for (const hour12 of [true, false]) {
    const options = withClockOptions(
      { dateStyle: "medium", timeStyle: "short" },
      hour12,
    );
    assert.doesNotThrow(() => new Intl.DateTimeFormat("en-US", options));
  }
});

test("createClockFormatter renders both clocks for the same instant", () => {
  resetPreference();
  const formatClockTime = createClockFormatter("en-US", {
    hour: "numeric",
    minute: "2-digit",
  });

  // Day-period markers use a narrow no-break space in modern ICU; normalize
  // it (`\s` covers U+00A0 and U+202F) so the assertion stays readable.
  const afternoon = formatClockTime(AFTERNOON).replace(/\s+/g, " ");
  assert.equal(afternoon, "2:34 PM");

  setTimeFormatPreference("24-hour");
  assert.equal(formatClockTime(AFTERNOON), "14:34");

  resetPreference();
});

test("createClockFormatter rebuilds when the preference flips back", () => {
  resetPreference();
  const formatClockTime = createClockFormatter("en-US", {
    hour: "numeric",
    minute: "2-digit",
  });

  const first = formatClockTime(AFTERNOON);
  setTimeFormatPreference("24-hour");
  const second = formatClockTime(AFTERNOON);
  resetPreference();
  const third = formatClockTime(AFTERNOON);

  assert.notEqual(first, second);
  assert.equal(first, third);
});

test("24-hour midnight reads 00:07, never 24:07", () => {
  setTimeFormatPreference("24-hour");
  const formatClockTime = createClockFormatter("en-US", {
    hour: "numeric",
    minute: "2-digit",
  });

  assert.equal(formatClockTime(MIDNIGHT), "00:07");
  resetPreference();
});

test("24-hour keeps en-US date wording and only swaps the clock", () => {
  setTimeFormatPreference("24-hour");
  const formatFullDateTime = createClockFormatter("en-US", {
    weekday: "long",
    year: "numeric",
    month: "long",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });

  const formatted = formatFullDateTime(AFTERNOON);
  assert.match(formatted, /Thursday, April 2, 2026/);
  assert.match(formatted, /14:34/);
  assert.doesNotMatch(formatted, /AM|PM/);

  resetPreference();
});
