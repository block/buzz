import assert from "node:assert/strict";
import test from "node:test";

import { settingsNavGroups } from "./SettingsView.tsx";
import { isSettingsSection, settingsSections } from "./SettingsPanels.tsx";

test("plugins-mcp is recognized as a valid settings section", () => {
  assert.ok(
    isSettingsSection("plugins-mcp"),
    'isSettingsSection("plugins-mcp") should be true',
  );
});

test("plugins-mcp is defined in settingsSections descriptors", () => {
  const descriptor = settingsSections.find((s) => s.value === "plugins-mcp");
  assert.ok(descriptor, "plugins-mcp descriptor must exist");
  assert.equal(descriptor.label, "Plugins & MCP");
  assert.ok(descriptor.icon, "plugins-mcp descriptor must have an icon");
});

test("plugins-mcp is wired into the App nav group", () => {
  const appGroup = settingsNavGroups.find((g) => g.label === "App");
  assert.ok(appGroup, "App group must exist in settingsNavGroups");
  assert.ok(
    appGroup.sections.includes("plugins-mcp"),
    `expected "plugins-mcp" in App group sections, got: ${JSON.stringify(appGroup.sections)}`,
  );
});

test("plugins-mcp follows agents in the App nav group", () => {
  const appGroup = settingsNavGroups.find((g) => g.label === "App");
  assert.ok(appGroup, "App group must exist");
  const agentsIndex = appGroup.sections.indexOf("agents");
  const pluginsIndex = appGroup.sections.indexOf("plugins-mcp");
  assert.ok(agentsIndex !== -1, "agents must be present in App group");
  assert.ok(pluginsIndex !== -1, "plugins-mcp must be present in App group");
  assert.ok(
    pluginsIndex > agentsIndex,
    `expected "plugins-mcp" after "agents", got: ${JSON.stringify(appGroup.sections)}`,
  );
});
