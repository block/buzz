/**
 * The same day ladder in the other shipped language (spec T-006 / AC-006):
 * the words and the `Intl` shapes follow zh-Hans while the business bands —
 * Today, Yesterday, the weekday window, the year cutoff — stay exactly where
 * `datetime.test.mjs` pins them in English.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { initializeI18n } from "@/i18n";
import { setLanguagePreference } from "@/i18n/language";
import { formatDayGroupLabel, formatItemTimestamp } from "./datetime.ts";

Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();

/** Local-time unix seconds, as in `datetime.test.mjs`. */
function at(year, monthIndex, day, hour = 12, minute = 0) {
  return new Date(year, monthIndex, day, hour, minute).getTime() / 1_000;
}

const NOW = at(2026, 6, 30, 14, 30); // Thu Jul 30 2026, 2:30 PM local

function speak(language) {
  // The user-facing switch: it moves the language module, which drives both
  // the catalogs and the formatter locale.
  setLanguagePreference(language);
}

function tick() {
  return new Promise((resolve) => {
    setTimeout(resolve, 0);
  });
}

test("zh-Hans labels the near bands with the catalog words", async () => {
  speak("zh-Hans");
  await tick();
  assert.equal(formatDayGroupLabel(at(2026, 6, 30, 9, 5), NOW), "今天");
  assert.equal(formatDayGroupLabel(at(2026, 6, 29, 23, 59), NOW), "昨天");
  speak("en");
  await tick();
});

test("the weekday and date rungs stop being English, bands unchanged", async () => {
  speak("zh-Hans");
  await tick();
  // Two days back is still the weekday rung, now 星期二 rather than Tuesday.
  const weekday = formatDayGroupLabel(at(2026, 6, 28), NOW);
  assert.equal(weekday, "星期二");
  // Seven days back leaves the weekday band for a dated label, and eight
  // months back still carries the year: the same rungs as in English.
  const dated = formatDayGroupLabel(at(2026, 6, 20), NOW);
  assert.match(dated, /月/);
  assert.match(dated, /日/);
  assert.doesNotMatch(dated, /[A-Za-z]/, `English month leaked: ${dated}`);
  assert.match(formatDayGroupLabel(at(2025, 5, 20), NOW), /2025/);
  speak("en");
  await tick();
});

test("a zh-Hans clock has no English day period", async () => {
  speak("zh-Hans");
  await tick();
  const today = at(2026, 6, 30, 14, 34);
  const clock = formatItemTimestamp(today, { nowSeconds: NOW });
  assert.match(clock, /^\d{1,2}:\d{2}$/, `unexpected clock "${clock}"`);
  assert.doesNotMatch(clock, /[AP]\.?M\.?/i);
  speak("en");
  await tick();
});

test("zh-Hans joins day and time without the English preposition", async () => {
  speak("zh-Hans");
  await tick();
  const label = formatItemTimestamp(at(2026, 6, 29, 14, 34), {
    withTime: true,
    nowSeconds: NOW,
  });
  assert.match(label, /^昨天/);
  assert.doesNotMatch(label, / at /, `English joiner leaked: "${label}"`);
  assert.match(label, /:\d{2}$/);
  // Compact rows outside today still drop the clock entirely.
  assert.equal(
    formatItemTimestamp(at(2026, 6, 29, 14, 34), { nowSeconds: NOW }),
    "昨天",
  );
  speak("en");
  await tick();
});

test("the labels follow a switch in both directions, never a load-time snapshot", async () => {
  // A module-level formatter would freeze on whichever locale the process
  // booted with, and this is the assertion that catches it.
  const probe = at(2026, 6, 29, 14, 34);
  const options = { withTime: true, nowSeconds: NOW };
  assert.equal(formatItemTimestamp(probe, options), "Yesterday at 2:34 PM");
  speak("zh-Hans");
  await tick();
  assert.match(formatItemTimestamp(probe, options), /^昨天/);
  speak("en");
  await tick();
  assert.equal(
    formatItemTimestamp(probe, options),
    "Yesterday at 2:34 PM",
    "English output must return byte-exact after a switch (NFR-008)",
  );
});
