import assert from "node:assert/strict";
import test from "node:test";

import {
  activateWelcomeTeamPersonasSequentially,
  LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT,
  pickWelcomeGuideAgent,
  pickWelcomeGuideAgentForRelay,
  pickWelcomeTeamStarterAgentForRelay,
  WELCOME_GUIDE_AGENT_NAME,
  WELCOME_GUIDE_PERSONA_ID,
  WELCOME_TEAM_ID,
  WELCOME_TEAM_STARTERS,
} from "./welcomeGuide.ts";

const PUB_A = "a".repeat(64);
const PUB_B = "b".repeat(64);
const PUB_C = "c".repeat(64);
const RELAY_A = "ws://localhost:3000";
const RELAY_B = "ws://localhost:3001";

function makeAgent(overrides = {}) {
  return {
    pubkey: PUB_A,
    name: WELCOME_GUIDE_AGENT_NAME,
    personaId: null,
    relayUrl: RELAY_A,
    acpCommand: "buzz-acp",
    agentCommand: "buzz-agent",
    agentArgs: [],
    mcpCommand: "buzz-dev-mcp",
    turnTimeoutSeconds: 120,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    model: null,
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: "2026-06-11T00:00:00.000Z",
    updatedAt: "2026-06-11T00:00:00.000Z",
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: "",
    startOnAppLaunch: false,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    teamId: WELCOME_TEAM_ID,
    ...overrides,
  };
}

test("pickWelcomeGuideAgent reuses a legacy Kit guide", () => {
  const legacyKit = makeAgent({
    name: "Kit",
    pubkey: PUB_A,
    systemPrompt: LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT,
  });

  assert.equal(pickWelcomeGuideAgent([legacyKit]), legacyKit);
});

test("pickWelcomeGuideAgent prefers a running legacy guide over stopped builtin Fizz", () => {
  const stoppedBuiltinFizz = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    status: "stopped",
  });
  const runningLegacyKit = makeAgent({
    name: "Kit",
    pubkey: PUB_B,
    status: "running",
    systemPrompt: LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT,
  });

  assert.equal(
    pickWelcomeGuideAgent([stoppedBuiltinFizz, runningLegacyKit]),
    runningLegacyKit,
  );
});

test("pickWelcomeGuideAgent ignores non-Kit agents with the legacy prompt", () => {
  const nonKit = makeAgent({
    pubkey: PUB_A,
    name: "Scout",
    systemPrompt: LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT,
  });
  const fizz = makeAgent({
    pubkey: PUB_C,
    personaId: WELCOME_GUIDE_PERSONA_ID,
  });

  assert.equal(pickWelcomeGuideAgent([nonKit, fizz]), fizz);
});

test("pickWelcomeGuideAgentForRelay ignores Fizz agents from other communities", () => {
  const otherCommunityFizz = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: RELAY_A,
    status: "running",
  });
  const currentCommunityFizz = makeAgent({
    pubkey: PUB_B,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: RELAY_B,
    status: "stopped",
  });

  assert.equal(
    pickWelcomeGuideAgentForRelay(
      [otherCommunityFizz, currentCommunityFizz],
      RELAY_B,
    ),
    currentCommunityFizz,
  );
});

test("pickWelcomeGuideAgentForRelay returns null when Fizz only exists in another community", () => {
  const otherCommunityFizz = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: RELAY_A,
  });

  assert.equal(
    pickWelcomeGuideAgentForRelay([otherCommunityFizz], RELAY_B),
    null,
  );
});

test("starter persona activation is serialized to protect the shared store", async () => {
  const calls = [];
  let activeWrites = 0;

  await activateWelcomeTeamPersonasSequentially(
    ["builtin:fizz", "builtin:honey", "builtin:bumble"],
    async (personaId) => {
      assert.equal(activeWrites, 0, "activation writes must never overlap");
      activeWrites += 1;
      calls.push(personaId);
      await new Promise((resolve) => setTimeout(resolve, 1));
      activeWrites -= 1;
    },
  );

  assert.deepEqual(calls, ["builtin:fizz", "builtin:honey", "builtin:bumble"]);
});

test("welcome team starter definitions and role identities are stable", () => {
  assert.equal(WELCOME_TEAM_ID, "builtin-team:welcome");
  assert.deepEqual(WELCOME_TEAM_STARTERS, [
    { name: "Fizz", personaId: "builtin:fizz", role: "lead" },
    { name: "Honey", personaId: "builtin:honey", role: "teammate" },
    { name: "Bumble", personaId: "builtin:bumble", role: "teammate" },
  ]);
});

test("starter matching ignores user agents with a Welcome persona", () => {
  const honey = WELCOME_TEAM_STARTERS[1];
  const userHoney = makeAgent({
    personaId: honey.personaId,
    teamId: null,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([userHoney], honey, RELAY_A),
    null,
  );
});

test("starter matching uses persona identity rather than display name", () => {
  const honey = WELCOME_TEAM_STARTERS[1];
  const renamedHoney = makeAgent({
    name: "Honey the Helper",
    personaId: honey.personaId,
  });
  const nameOnlyHoney = makeAgent({ name: honey.name, pubkey: PUB_B });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [nameOnlyHoney, renamedHoney],
      honey,
      RELAY_A,
    ),
    renamedHoney,
  );
});

test("starter matching is relay scoped and normalizes trailing slashes", () => {
  const bumble = WELCOME_TEAM_STARTERS[2];
  const otherRelay = makeAgent({
    personaId: bumble.personaId,
    relayUrl: RELAY_B,
    status: "running",
  });
  const matchingRelay = makeAgent({
    personaId: bumble.personaId,
    relayUrl: `${RELAY_A}/`,
    pubkey: PUB_B,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [otherRelay, matchingRelay],
      bumble,
      RELAY_A,
    ),
    matchingRelay,
  );
});

test("starter matching prefers running, then deployed instances", () => {
  const fizz = WELCOME_TEAM_STARTERS[0];
  const stopped = makeAgent({ personaId: fizz.personaId });
  const deployed = makeAgent({
    personaId: fizz.personaId,
    pubkey: PUB_B,
    status: "deployed",
  });
  const running = makeAgent({
    personaId: fizz.personaId,
    pubkey: PUB_C,
    status: "running",
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [stopped, deployed, running],
      fizz,
      RELAY_A,
    ),
    running,
  );
  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([stopped, deployed], fizz, RELAY_A),
    deployed,
  );
});
