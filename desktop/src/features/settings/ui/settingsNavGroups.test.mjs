import assert from "node:assert/strict";
import test from "node:test";

import { settingsNavGroups } from "./SettingsView.tsx";

test("relay-admin is wired into the Communities nav group", () => {
  const communitiesGroup = settingsNavGroups.find(
    (g) => g.label === "Communities",
  );
  assert.ok(
    communitiesGroup,
    "Communities group must exist in settingsNavGroups",
  );
  assert.ok(
    communitiesGroup.sections.includes("relay-admin"),
    `expected "relay-admin" in Communities group sections, got: ${JSON.stringify(communitiesGroup.sections)}`,
  );
});

test("relay-admin follows community-members in the Communities nav group", () => {
  const communitiesGroup = settingsNavGroups.find(
    (g) => g.label === "Communities",
  );
  assert.ok(
    communitiesGroup,
    "Communities group must exist in settingsNavGroups",
  );
  const membersIndex = communitiesGroup.sections.indexOf("community-members");
  const relayAdminIndex = communitiesGroup.sections.indexOf("relay-admin");
  assert.ok(membersIndex !== -1, "community-members must be present");
  assert.ok(
    relayAdminIndex > membersIndex,
    `expected "relay-admin" after "community-members", got: ${JSON.stringify(communitiesGroup.sections)}`,
  );
});

test("the removed admin-console id is not wired into any nav group", () => {
  for (const group of settingsNavGroups) {
    assert.ok(
      !group.sections.includes("admin-console"),
      `"admin-console" must not appear in the "${group.label}" group`,
    );
  }
});
