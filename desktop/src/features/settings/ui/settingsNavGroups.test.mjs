import assert from "node:assert/strict";
import test from "node:test";

import { settingsNavGroups } from "./SettingsView.tsx";

test("admin-console is present in the App nav group", () => {
  const appGroup = settingsNavGroups.find((g) => g.label === "App");
  assert.ok(appGroup, "App group must exist in settingsNavGroups");
  assert.ok(
    appGroup.sections.includes("admin-console"),
    `expected "admin-console" in App group sections, got: ${JSON.stringify(appGroup.sections)}`,
  );
});

test("admin-console is the last entry in the App nav group", () => {
  const appGroup = settingsNavGroups.find((g) => g.label === "App");
  assert.ok(appGroup, "App group must exist in settingsNavGroups");
  const last = appGroup.sections.at(-1);
  assert.equal(
    last,
    "admin-console",
    `expected "admin-console" to be last in App group, got: ${last}`,
  );
});
