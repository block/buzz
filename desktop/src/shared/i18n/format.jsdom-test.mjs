import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { setAppLanguage } from "../../i18n.ts";
import {
  formatTtlDuration,
  getEphemeralChannelDisplay,
  parseTtlDuration,
} from "../../features/channels/lib/ephemeralChannel.ts";
import { uiDateFormatter, uiRelativeTimeFormatter } from "./format.ts";

afterEach(async () => {
  await setAppLanguage("en");
  localStorage.clear();
});

test("cached date and relative formatters follow later language changes", async () => {
  const date = new Date("2026-10-24T06:30:00.000Z");
  const options = { month: "short", day: "numeric", timeZone: "UTC" };
  const absolute = uiDateFormatter(options);
  const relative = uiRelativeTimeFormatter({ numeric: "auto" });
  for (const [language, locale] of [
    ["en", "en-US"],
    ["ko", "ko-KR"],
    ["en", "en-US"],
  ]) {
    await setAppLanguage(language);
    assert.equal(
      absolute.format(date),
      new Intl.DateTimeFormat(locale, options).format(date),
    );
    assert.equal(
      relative.format(1, "day"),
      new Intl.RelativeTimeFormat(locale, { numeric: "auto" }).format(1, "day"),
    );
  }
  assert.equal(date.toISOString(), "2026-10-24T06:30:00.000Z");
});

test("ephemeral channel status localizes future, missing, and expired deadlines without changing TTL meaning", async () => {
  const now = Date.parse("2026-10-24T06:30:00.000Z");
  const future = {
    ttlSeconds: 7200,
    ttlDeadline: new Date(now + 7200000).toISOString(),
  };
  const original = structuredClone(future);
  await setAppLanguage("en");
  assert.equal(getEphemeralChannelDisplay(future, now).detailLabel, "2h left");
  await setAppLanguage("ko");
  const korean = getEphemeralChannelDisplay(future, now);
  assert.equal(korean.detailLabel, "2시간 남음");
  assert.ok(!korean.tooltipLabel.includes("Ephemeral channel"));
  assert.ok(korean.tooltipLabel.includes("2시간"));
  const noDeadline = getEphemeralChannelDisplay(
    { ttlSeconds: 120, ttlDeadline: null },
    now,
  );
  assert.ok(noDeadline.tooltipLabel.includes("2분"));
  const expired = getEphemeralChannelDisplay(
    { ...future, ttlDeadline: new Date(now - 1000).toISOString() },
    now,
  );
  assert.ok(!expired.tooltipLabel.includes("예정"));
  assert.ok(!expired.tooltipLabel.includes("Ephemeral"));
  assert.deepEqual(future, original);
  assert.equal(parseTtlDuration("1d12h"), 129600);
  assert.equal(formatTtlDuration(129600), "1d12h");
  await setAppLanguage("en");
  assert.equal(
    getEphemeralChannelDisplay(
      { ...future, ttlDeadline: new Date(now - 1000).toISOString() },
      now,
    ).tooltipLabel,
    "Ephemeral channel. Cleanup is due now.",
  );
});

test("forum and project time displays follow language while preserving their timestamps", async () => {
  const { formatRelativeTime } = await import(
    "../../features/forum/lib/time.ts"
  );
  const { relativeTime, formatExactTimestamp } = await import(
    "../../features/projects/lib/projectsViewHelpers.ts"
  );
  const now = Math.floor(Date.now() / 1000);
  for (const language of ["en", "ko", "en"]) {
    await setAppLanguage(language);
    assert.equal(
      formatRelativeTime(now),
      language === "ko" ? "방금" : "just now",
    );
    assert.equal(
      formatRelativeTime(now - 120),
      language === "ko" ? "2분 전" : "2m ago",
    );
    assert.equal(
      relativeTime(now - 120, now),
      language === "ko" ? "2분 전" : "2 minutes ago",
    );
    assert.equal(
      formatExactTimestamp(now),
      new Intl.DateTimeFormat(language === "ko" ? "ko-KR" : "en-US", {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
        second: "2-digit",
      }).format(now * 1000),
    );
  }
});
