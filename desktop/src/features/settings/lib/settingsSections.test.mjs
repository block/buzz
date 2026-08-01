import assert from "node:assert/strict";
import { test } from "node:test";

import {
  DEFAULT_SETTINGS_SECTION,
  isSettingsSection,
} from "./settingsSections.ts";

// Every section that has a nav descriptor and a render case in
// SettingsPanels.tsx must be accepted by isSettingsSection — otherwise the
// /settings?section=<value> route search validation strips the param and the
// panel never opens (regression: "stickers" was routable in the nav but
// rejected by the guard, so clicking Stickers fell back to the default
// section).
const EXPECTED_SECTIONS = [
  "profile",
  "notifications",
  "experimental",
  "agents",
  "channel-templates",
  "compute",
  "appearance",
  "shortcuts",
  "hosted-communities",
  "community-members",
  "moderation",
  "custom-emoji",
  "stickers",
  "local-archive",
  "mobile",
  "updates",
];

test("isSettingsSection accepts every section with a nav entry", () => {
  for (const section of EXPECTED_SECTIONS) {
    assert.equal(isSettingsSection(section), true, `section "${section}"`);
  }
});

test("isSettingsSection rejects unknown and non-string values", () => {
  assert.equal(isSettingsSection("nope"), false);
  assert.equal(isSettingsSection(""), false);
  assert.equal(isSettingsSection(undefined), false);
  assert.equal(isSettingsSection(42), false);
});

test("default settings section is a valid section", () => {
  assert.equal(isSettingsSection(DEFAULT_SETTINGS_SECTION), true);
});
