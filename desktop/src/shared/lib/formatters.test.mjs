/**
 * The locale-aware formatter factory (spec FR-006 / NFR-008): one cached
 * formatter per (locale, options) pair, the locale read from i18n at call
 * time, and display words that come from the catalogs.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { initializeI18n } from "@/i18n";
import {
  appIntlLocale,
  appLanguage,
  dateTimeFormatter,
  dateWords,
  fillTemplate,
  formatterCacheSize,
  intlTagForLanguage,
  joinDayAndTime,
} from "./formatters.ts";

Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();

const TIME_OPTIONS = Object.freeze({ hour: "numeric", minute: "2-digit" });
const WEEKDAY_OPTIONS = Object.freeze({ weekday: "long" });

async function speak(language) {
  const { i18n } = await import("@/i18n");
  await i18n.changeLanguage(language);
}

test("en formats with the tag the hard-coded formatters used", () => {
  assert.equal(appLanguage("en"), "en");
  assert.equal(appLanguage("zh-Hans"), "zh-Hans");
  // Uninitialized / unexpected values resolve to the fallback language.
  assert.equal(appLanguage(undefined), "en");
  assert.equal(appLanguage("fr"), "en");
  assert.equal(intlTagForLanguage("en"), "en-US");
  assert.equal(appIntlLocale(), "en-US");
});

test("zh-Hans resolves a Chinese tag instead of silently formatting in en", () => {
  const tag = intlTagForLanguage("zh-Hans");
  assert.match(tag, /^zh/, `ICU gave ${tag}, not a Chinese locale`);
});

test("one (locale, options) pair yields the same formatter instance", () => {
  const first = dateTimeFormatter(TIME_OPTIONS);
  assert.equal(dateTimeFormatter(TIME_OPTIONS), first);
  assert.equal(dateTimeFormatter({ ...TIME_OPTIONS }), first);
  assert.notEqual(dateTimeFormatter(WEEKDAY_OPTIONS), first);
  // Same preset, other locale: a second instance, never a mutation of the en.
  const zh = dateTimeFormatter(TIME_OPTIONS, "zh-Hans");
  assert.notEqual(zh, first);
  assert.equal(dateTimeFormatter(TIME_OPTIONS), first);
});

test("the cache grows per pair, not per render", async () => {
  const before = formatterCacheSize();
  for (let index = 0; index < 500; index += 1) {
    dateTimeFormatter(TIME_OPTIONS).format(new Date(2026, 6, 30, 14, 34));
    dateTimeFormatter(WEEKDAY_OPTIONS).format(new Date(2026, 6, 30, 14, 34));
  }
  await speak("zh-Hans");
  for (let index = 0; index < 500; index += 1) {
    dateTimeFormatter(TIME_OPTIONS).format(new Date(2026, 6, 30, 14, 34));
  }
  await speak("en");
  // en presets were already built; only the zh-Hans pair is new. One entry
  // per distinct pair is the whole point of caching (NFR-006).
  assert.ok(
    formatterCacheSize() - before <= 3,
    `cache grew by ${formatterCacheSize() - before}`,
  );
});

test("English words are byte-identical to the strings the catalogs replaced", async () => {
  assert.deepEqual(dateWords(), { today: "Today", yesterday: "Yesterday" });
  assert.equal(joinDayAndTime("Yesterday", "2:34 PM"), "Yesterday at 2:34 PM");
  await speak("zh-Hans");
  assert.deepEqual(dateWords(), { today: "今天", yesterday: "昨天" });
  // zh needs no preposition; the template is the translatable unit (§4.4).
  assert.equal(joinDayAndTime("昨天", "14:34"), "昨天 14:34");
  await speak("en");
});

test("unresolved placeholders stay visible instead of vanishing", () => {
  assert.equal(fillTemplate("{{a}}/{{b}}", { a: "1", b: "2" }), "1/2");
  assert.equal(
    fillTemplate("{{date}} at {{time}}", { date: "Today" }),
    "Today at {{time}}",
  );
});
