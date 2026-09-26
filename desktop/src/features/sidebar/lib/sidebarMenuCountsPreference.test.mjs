import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
  removeItem: (key) => values.delete(key),
};

const preference = await import("./sidebarMenuCountsPreference.ts");

test("defaults invalid and missing sidebar menu count prefs to all off", () => {
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences(null),
    preference.DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES,
  );
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences("yes"),
    preference.DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES,
  );
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences("{"),
    preference.DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES,
  );
  assert.equal(preference.DEFAULT_SIDEBAR_MENU_COUNTS_ENABLED, false);
});

test("parses per-item prefs and ignores unknown keys", () => {
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences(
      JSON.stringify({ inbox: true, bots: true, extra: true }),
    ),
    {
      inbox: true,
      browsers: false,
      agents: false,
      bots: true,
    },
  );
});

test("migrates legacy global true/false into per-item prefs", () => {
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences(null, "true"),
    {
      inbox: true,
      browsers: true,
      agents: true,
      bots: true,
    },
  );
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences(null, "false"),
    {
      inbox: false,
      browsers: false,
      agents: false,
      bots: false,
    },
  );
  // Explicit per-item JSON wins over legacy global.
  assert.deepEqual(
    preference.parseSidebarMenuCountPreferences(
      JSON.stringify({ inbox: true }),
      "true",
    ),
    {
      inbox: true,
      browsers: false,
      agents: false,
      bots: false,
    },
  );
});

test("persists individual sidebar menu count preferences", () => {
  values.clear();
  preference.setSidebarMenuCountPreference("inbox", true);
  assert.equal(preference.getSidebarMenuCountPreference("inbox"), true);
  assert.equal(preference.getSidebarMenuCountPreference("bots"), false);
  assert.equal(
    values.has(preference.SIDEBAR_MENU_COUNTS_STORAGE_KEY),
    false,
  );
  const stored = JSON.parse(
    values.get(preference.SIDEBAR_MENU_COUNT_PREFS_STORAGE_KEY),
  );
  assert.equal(stored.inbox, true);
  assert.equal(stored.bots, false);

  preference.setSidebarMenuCountPreference("bots", true);
  assert.equal(preference.getSidebarMenuCountPreference("bots"), true);
  assert.equal(preference.getSidebarMenuCountsEnabled(), false);

  preference.setSidebarMenuCountsEnabled(true);
  assert.equal(preference.getSidebarMenuCountsEnabled(), true);
  assert.deepEqual(preference.getSidebarMenuCountPreferences(), {
    inbox: true,
    browsers: true,
    agents: true,
    bots: true,
  });

  preference.setSidebarMenuCountsEnabled(false);
  assert.equal(preference.getSidebarMenuCountsEnabled(), false);
  assert.deepEqual(
    preference.getSidebarMenuCountPreferences(),
    preference.DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES,
  );
});

test("legacy parseSidebarMenuCountsEnabled still accepts true/false", () => {
  assert.equal(preference.parseSidebarMenuCountsEnabled(null), false);
  assert.equal(preference.parseSidebarMenuCountsEnabled("yes"), false);
  assert.equal(preference.parseSidebarMenuCountsEnabled("false"), false);
  assert.equal(preference.parseSidebarMenuCountsEnabled("true"), true);
});
