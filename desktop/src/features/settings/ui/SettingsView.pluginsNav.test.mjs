/**
 * `settingsNavGroups` (SettingsView.tsx) had a "plugins" descriptor in
 * `settingsSections` (SettingsPanels.tsx) — a `SettingsSection` value, a
 * render case, and no feature gate — but no group ever listed "plugins" in
 * its `sections` array, so nothing ever rendered a nav button for it: the
 * panel existed but was unreachable from the real app. Pins both halves of
 * that contract so a future regression can't silently drop it again.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { settingsNavGroups } from "./SettingsView.tsx";
import { settingsSections } from "./SettingsPanels.tsx";

test("plugins is listed in exactly one nav group", () => {
  const groupsWithPlugins = settingsNavGroups.filter((group) =>
    group.sections.includes("plugins"),
  );
  assert.equal(
    groupsWithPlugins.length,
    1,
    "plugins must appear in exactly one settings nav group",
  );
});

test("every section listed in a nav group has a matching descriptor — plugins included", () => {
  const descriptorValues = new Set(settingsSections.map((s) => s.value));
  for (const group of settingsNavGroups) {
    for (const value of group.sections) {
      assert.ok(
        descriptorValues.has(value),
        `nav group "${group.label}" lists "${value}" but no descriptor exists for it — it would silently render nothing`,
      );
    }
  }
  assert.ok(descriptorValues.has("plugins"));
});

test("the plugins descriptor has no feature gate — it renders unconditionally", () => {
  const plugins = settingsSections.find((s) => s.value === "plugins");
  assert.ok(plugins, "plugins descriptor must exist");
  assert.equal(
    plugins.featureGate,
    undefined,
    "a feature gate here would make plugins invisible by default, defeating this fix",
  );
});
