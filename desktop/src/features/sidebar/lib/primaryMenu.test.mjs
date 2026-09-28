import assert from "node:assert/strict";
import test from "node:test";

import {
  PRIMARY_MENU_ITEMS,
  primaryMenuItemAfter,
  primaryMenuLabels,
} from "./primaryMenu.ts";

test("primary menu order: Agents, Bots, then Browsers (Assistant is injected in chrome)", () => {
  assert.deepEqual(primaryMenuLabels(), [
    "Inbox",
    "Pulse",
    "Projects",
    "Agents",
    "Bots",
    "Browsers",
    "Workflows",
  ]);
  assert.equal(primaryMenuItemAfter("agents").id, "bots");
  assert.equal(primaryMenuItemAfter("bots").id, "browsers");
  assert.equal(primaryMenuItemAfter("bots").label, "Browsers");
  assert.equal(primaryMenuItemAfter("bots").testId, "open-browsers-view");
  assert.equal(primaryMenuItemAfter("browsers").id, "workflows");

  const agentsIndex = PRIMARY_MENU_ITEMS.findIndex(
    (item) => item.id === "agents",
  );
  const botsIndex = PRIMARY_MENU_ITEMS.findIndex((item) => item.id === "bots");
  const browsersIndex = PRIMARY_MENU_ITEMS.findIndex(
    (item) => item.id === "browsers",
  );
  assert.equal(botsIndex, agentsIndex + 1);
  assert.equal(browsersIndex, botsIndex + 1);
  assert.ok(
    browsersIndex <
      PRIMARY_MENU_ITEMS.findIndex((item) => item.id === "workflows"),
  );
});

test("Bots is its own primary item, not mixed into Agents or Settings", () => {
  const bots = PRIMARY_MENU_ITEMS.find((item) => item.id === "bots");
  const agents = PRIMARY_MENU_ITEMS.find((item) => item.id === "agents");
  assert.ok(bots);
  assert.ok(agents);
  assert.notEqual(bots.testId, agents.testId);
  assert.notEqual(bots.label, "Settings");
});
