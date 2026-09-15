import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { ensureWelcomeTeam } from "./welcomeGuide.ts";

globalThis.window = {};
const starterIds = ["builtin:fizz", "builtin:honey", "builtin:bumble"];
const welcomeTeam = {
  id: "builtin-team:welcome",
  name: "Welcome Team",
  persona_ids: starterIds,
};

afterEach(() => {
  clearMocks();
});

test("Welcome bootstrap respects a removed starter instead of reactivating it", async () => {
  const calls = [];
  mockIPC((command) => {
    calls.push(command);
    if (command === "list_teams") return [welcomeTeam];
    if (command === "list_managed_agents") return [];
    if (command === "list_personas")
      return starterIds.map((id) => ({
        id,
        display_name: id,
        is_builtin: true,
        is_active: id !== "builtin:fizz",
      }));
    throw new Error(`Unexpected mutation or discovery: ${command}`);
  });
  assert.equal(
    await ensureWelcomeTeam("welcome", "ws://isolated.invalid"),
    null,
  );
  assert.deepEqual(calls, ["list_teams", "list_personas"]);
});

test("Welcome bootstrap does not provision a removed or unselected team", async () => {
  const calls = [];
  mockIPC((command) => {
    calls.push(command);
    if (command === "list_teams" || command === "list_managed_agents")
      return [];
    if (command === "list_personas") return [];
    throw new Error(`Unexpected mutation or discovery: ${command}`);
  });
  assert.equal(
    await ensureWelcomeTeam("welcome", "ws://isolated.invalid"),
    null,
  );
  assert.deepEqual(calls, ["list_teams"]);
});
