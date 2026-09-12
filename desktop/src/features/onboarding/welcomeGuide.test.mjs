import assert from "node:assert/strict";
import test from "node:test";

import {
  activateWelcomeTeamPersonasSequentially,
  buildWelcomeStarterCreateInput,
  LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT,
  pickWelcomeGuideAgent,
  pickWelcomeGuideAgentForRelay,
  pickWelcomeTeamStarterAgentForRelay,
  RETIRED_WELCOME_FIZZ_TEAM_ID,
  welcomeStarterRuntimeUpdate,
  welcomeTeammateAccessUpdate,
  welcomeTeammateHasExpectedAccess,
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
    agentCommandOverride: null,
    agentArgs: [],
    mcpCommand: "buzz-dev-mcp",
    turnTimeoutSeconds: 120,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    model: null,
    provider: null,
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

test("pickWelcomeGuideAgentForRelay prefers Fizz pinned to the target community", () => {
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

test("a relay pin matches its backend-equivalent spellings", () => {
  // The backend's pair key folds host case, loopback aliases and default
  // ports. Trimming alone treated each spelling as a separate community.
  //
  // A lone candidate is returned whatever its rank — `pickAgentForRelay` falls
  // through every rank rather than reporting a starter missing — so a
  // single-agent assertion here would pass with or without canonicalization.
  // Each case therefore offers a DECOY listed first: unless the equivalent
  // spelling is recognised as rank 0, the decoy wins on array order.
  const equivalent = [
    ["ws://localhost:3000", "ws://127.0.0.1:3000"],
    ["wss://RELAY.EXAMPLE:443/", "wss://relay.example"],
    ["ws://relay.example:80", "ws://relay.example"],
    ["wss://relay.example/", "wss://relay.example"],
  ];

  for (const [pinned, target] of equivalent) {
    const decoy = makeAgent({
      pubkey: PUB_B,
      personaId: WELCOME_GUIDE_PERSONA_ID,
      relayUrl: "wss://unrelated.example",
    });
    const match = makeAgent({
      pubkey: PUB_A,
      personaId: WELCOME_GUIDE_PERSONA_ID,
      relayUrl: pinned,
    });

    assert.equal(
      pickWelcomeGuideAgentForRelay([decoy, match], target),
      match,
      `${pinned} should rank as pinned to ${target}`,
    );
  }
});

test("a genuinely different relay is still a different community", () => {
  // Canonicalization must fold equivalent spellings without collapsing
  // distinct hosts — the failure mode in the other direction.
  const pinnedElsewhere = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: "wss://relay.example",
  });
  const pinnedHere = makeAgent({
    pubkey: PUB_B,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: "wss://other.example",
  });

  assert.equal(
    pickWelcomeGuideAgentForRelay(
      [pinnedElsewhere, pinnedHere],
      "wss://other.example",
    ),
    pinnedHere,
  );
});

test("a malformed legacy pin still matches itself", () => {
  // canonicalRelayUrl returns null for anything that is not a ws/wss URL. Two
  // records carrying the same unparseable pin must still rank as the same
  // community rather than each becoming its own. Decoy first again, so the
  // assertion fails if the malformed pin is not recognised as a match.
  const decoy = makeAgent({
    pubkey: PUB_B,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: "wss://unrelated.example",
  });
  const legacy = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: "not a url",
  });

  assert.equal(
    pickWelcomeGuideAgentForRelay([decoy, legacy], "NOT A URL "),
    legacy,
  );
});

test("pickWelcomeGuideAgentForRelay reuses Fizz from another community", () => {
  const otherCommunityFizz = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: RELAY_A,
  });

  assert.equal(
    pickWelcomeGuideAgentForRelay([otherCommunityFizz], RELAY_B),
    otherCommunityFizz,
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

test("all Welcome starters use the onboarding runtime preference", async () => {
  const claude = {
    id: "claude",
    label: "Claude",
    avatarUrl: "https://runtime/claude.png",
    availability: "available",
    command: "claude-code-acp",
    binaryPath: "/bin/claude-code-acp",
    defaultArgs: [],
    mcpCommand: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: "/bin/claude",
  };
  const buzzAgent = {
    ...claude,
    id: "buzz-agent",
    label: "Buzz Agent",
    command: "buzz-agent",
  };

  for (const starter of WELCOME_TEAM_STARTERS) {
    const input = await buildWelcomeStarterCreateInput(
      starter,
      {
        id: starter.personaId,
        displayName: starter.name,
        systemPrompt: `${starter.name} prompt`,
        model: null,
        provider: null,
        runtime: null,
        avatarUrl: null,
        envVars: {},
        isBuiltIn: true,
        isActive: true,
      },
      [buzzAgent, claude],
      "claude",
      RELAY_A,
    );

    assert.equal(input.agentCommand, "claude-code-acp");
    assert.equal(input.harnessOverride, true);
    assert.equal(input.personaId, starter.personaId);
    assert.equal(input.teamId, WELCOME_TEAM_ID);
    assert.equal(input.relayUrl, RELAY_A);
    assert.equal(input.spawnAfterCreate, false);
    assert.equal(input.startOnAppLaunch, false);
  }
});

test("existing Welcome starter rematerializes runtime-specific fields atomically", () => {
  const existing = makeAgent({
    pubkey: PUB_A,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    agentCommand: "claude-agent-acp",
    agentCommandOverride: "claude-agent-acp",
    agentArgs: ["--old"],
    mcpCommand: "",
    model: "claude-sonnet",
    provider: "anthropic",
  });

  assert.deepEqual(
    welcomeStarterRuntimeUpdate(existing, {
      name: "Fizz",
      agentCommand: "codex-acp",
      agentArgs: ["--new"],
      mcpCommand: "buzz-dev-mcp",
      model: "gpt-5.6-sol",
      provider: null,
    }),
    {
      pubkey: PUB_A,
      agentCommand: "codex-acp",
      harnessOverride: true,
      agentArgs: ["--new"],
      mcpCommand: "buzz-dev-mcp",
      model: "gpt-5.6-sol",
      provider: null,
    },
  );
});

test("existing Welcome starter clears stale model and provider for Claude", () => {
  const existing = makeAgent({
    personaId: WELCOME_GUIDE_PERSONA_ID,
    agentCommand: "codex-acp",
    agentArgs: [],
    model: "gpt-5.6-sol",
    provider: "openai",
  });

  assert.deepEqual(
    welcomeStarterRuntimeUpdate(existing, {
      name: "Fizz",
      agentCommand: "claude-agent-acp",
      agentArgs: [],
      mcpCommand: "",
    }),
    {
      pubkey: PUB_A,
      agentCommand: "claude-agent-acp",
      harnessOverride: true,
      agentArgs: [],
      mcpCommand: "",
      model: null,
      provider: null,
    },
  );
});

test("existing Welcome starter needs no update when runtime already matches", () => {
  const existing = makeAgent({
    personaId: WELCOME_GUIDE_PERSONA_ID,
    agentCommand: "codex-acp",
    agentArgs: ["--same"],
  });

  assert.equal(
    welcomeStarterRuntimeUpdate(existing, {
      name: "Fizz",
      agentCommand: "codex-acp",
      agentArgs: ["--same"],
      mcpCommand: "buzz-dev-mcp",
      model: null,
      provider: null,
    }),
    null,
  );
});

test("welcome team starter definitions and role identities are stable", () => {
  assert.equal(WELCOME_TEAM_ID, "builtin-team:welcome");
  assert.deepEqual(WELCOME_TEAM_STARTERS, [
    { name: "Fizz", personaId: "builtin:fizz", role: "lead" },
    { name: "Honey", personaId: "builtin:honey", role: "teammate" },
    { name: "Pollen", personaId: "builtin:bumble", role: "teammate" },
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
  const pollen = WELCOME_TEAM_STARTERS[2];
  const otherRelay = makeAgent({
    personaId: pollen.personaId,
    relayUrl: RELAY_B,
    status: "running",
  });
  const matchingRelay = makeAgent({
    personaId: pollen.personaId,
    relayUrl: `${RELAY_A}/`,
    pubkey: PUB_B,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [otherRelay, matchingRelay],
      pollen,
      RELAY_A,
    ),
    matchingRelay,
  );
});

test("starter matching reuses the existing instance in a second community", () => {
  // Regression: a pin miss used to fall through to createManagedAgent, which
  // mints a fresh keypair — so every community a user joined produced another
  // Bumble with a different pubkey.
  const bumble = WELCOME_TEAM_STARTERS[2];
  const firstCommunityBumble = makeAgent({
    personaId: bumble.personaId,
    relayUrl: RELAY_A,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [firstCommunityBumble],
      bumble,
      RELAY_B,
    ),
    firstCommunityBumble,
  );
});

test("starter matching reuses an unbound instance", () => {
  // The backend stores "" when no relay pin was supplied, and "" never equalled
  // any target relay — so unbound records matched nothing and always re-minted.
  const honey = WELCOME_TEAM_STARTERS[1];
  const unbound = makeAgent({ personaId: honey.personaId, relayUrl: "" });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([unbound], honey, RELAY_A),
    unbound,
  );
});

test("starter matching ranks pinned over unbound over another community", () => {
  const fizz = WELCOME_TEAM_STARTERS[0];
  const pinned = makeAgent({ personaId: fizz.personaId, relayUrl: RELAY_A });
  const unbound = makeAgent({
    personaId: fizz.personaId,
    pubkey: PUB_B,
    relayUrl: "",
  });
  const otherCommunity = makeAgent({
    personaId: fizz.personaId,
    pubkey: PUB_C,
    relayUrl: RELAY_B,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [otherCommunity, unbound, pinned],
      fizz,
      RELAY_A,
    ),
    pinned,
  );
  assert.equal(
    pickWelcomeTeamStarterAgentForRelay(
      [otherCommunity, unbound],
      fizz,
      RELAY_A,
    ),
    unbound,
  );
  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([otherCommunity], fizz, RELAY_A),
    otherCommunity,
  );
});

test("relay preference never overrides the Welcome Team scope", () => {
  // Falling through pin ranks must not start reusing a user's own agent that
  // merely shares the persona.
  const honey = WELCOME_TEAM_STARTERS[1];
  const userHoney = makeAgent({
    personaId: honey.personaId,
    teamId: null,
    relayUrl: RELAY_B,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([userHoney], honey, RELAY_A),
    null,
  );
});

test("starter matching reuses the retired built-in Fizz in a second community", () => {
  // Regression: records provisioned under the retired single-member built-in
  // Fizz team (#1718) carry teamId "builtin-team:fizz", not WELCOME_TEAM_ID,
  // so joining a second community fell through to createManagedAgent and
  // minted a duplicate Fizz keypair. A non-null pick here is what suppresses
  // the createManagedAgent call in provisionWelcomeTeam — if the retired
  // record stops matching, this assertion goes red.
  const fizz = WELCOME_TEAM_STARTERS[0];
  const retiredFizz = makeAgent({
    personaId: WELCOME_GUIDE_PERSONA_ID,
    teamId: RETIRED_WELCOME_FIZZ_TEAM_ID,
    relayUrl: RELAY_A,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([retiredFizz], fizz, RELAY_B),
    retiredFizz,
  );
});

test("the retired built-in Fizz record never satisfies a teammate starter", () => {
  const honey = WELCOME_TEAM_STARTERS[1];
  const retiredFizz = makeAgent({
    personaId: WELCOME_GUIDE_PERSONA_ID,
    teamId: RETIRED_WELCOME_FIZZ_TEAM_ID,
  });

  assert.equal(
    pickWelcomeTeamStarterAgentForRelay([retiredFizz], honey, RELAY_A),
    null,
  );
});

test("starter matching never absorbs a user agent that shares the Fizz persona", () => {
  // The retired-team match pins team id + persona + the stock name, so the
  // two ways a user-owned Fizz-persona agent can look similar both miss:
  // a renamed agent left in a demoted copy of the retired team, and an
  // agent named "Fizz" outside any built-in team.
  const fizz = WELCOME_TEAM_STARTERS[0];
  const renamedInRetiredTeam = makeAgent({
    name: "My Fizz",
    personaId: WELCOME_GUIDE_PERSONA_ID,
    teamId: RETIRED_WELCOME_FIZZ_TEAM_ID,
  });
  const stockNameOutsideTeam = makeAgent({
    name: WELCOME_GUIDE_AGENT_NAME,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    teamId: null,
  });

  for (const lookalike of [renamedInRetiredTeam, stockNameOutsideTeam]) {
    assert.equal(
      pickWelcomeTeamStarterAgentForRelay([lookalike], fizz, RELAY_A),
      null,
      `${lookalike.name} (teamId ${lookalike.teamId}) must not be reused`,
    );
  }
});

test("the retired built-in Fizz match pins the exact stock name", () => {
  // Mutation-sensitive: the identity check is `name === stockName`. A
  // case-folded or whitespace-padded comparison would absorb user-customized
  // records in a demoted retired team — both mutations must go red here.
  const fizz = WELCOME_TEAM_STARTERS[0];
  const caseFolded = makeAgent({
    name: WELCOME_GUIDE_AGENT_NAME.toUpperCase(),
    personaId: WELCOME_GUIDE_PERSONA_ID,
    teamId: RETIRED_WELCOME_FIZZ_TEAM_ID,
  });
  const padded = makeAgent({
    name: ` ${WELCOME_GUIDE_AGENT_NAME} `,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    teamId: RETIRED_WELCOME_FIZZ_TEAM_ID,
  });

  for (const customized of [caseFolded, padded]) {
    assert.equal(
      pickWelcomeTeamStarterAgentForRelay([customized], fizz, RELAY_A),
      null,
      `${JSON.stringify(customized.name)} must not be absorbed as the retired built-in Fizz`,
    );
  }
});

test("a credentialed or fragmented pin takes the malformed fallback, not an exact match", () => {
  // buzz-core rejects credentials and fragments outright; if the desktop
  // helper silently stripped them, `wss://user@relay.example` would
  // canonicalize to `wss://relay.example` and rank as pinned to that
  // community. The ambiguous record is listed first, so it wins on array
  // order the moment it is mis-ranked as an exact match.
  const ambiguousPins = ["wss://user@relay.example", "wss://relay.example/#x"];

  for (const pin of ambiguousPins) {
    const ambiguous = makeAgent({
      pubkey: PUB_A,
      personaId: WELCOME_GUIDE_PERSONA_ID,
      relayUrl: pin,
    });
    const unbound = makeAgent({
      pubkey: PUB_B,
      personaId: WELCOME_GUIDE_PERSONA_ID,
      relayUrl: "",
    });

    assert.equal(
      pickWelcomeGuideAgentForRelay(
        [ambiguous, unbound],
        "wss://relay.example",
      ),
      unbound,
      `${pin} must not rank as pinned to wss://relay.example`,
    );

    // The same malformed pin still matches itself via the stable fallback.
    const decoy = makeAgent({
      pubkey: PUB_C,
      personaId: WELCOME_GUIDE_PERSONA_ID,
      relayUrl: "wss://unrelated.example",
    });
    assert.equal(
      pickWelcomeGuideAgentForRelay([decoy, ambiguous], pin),
      ambiguous,
      `${pin} should still match its own spelling`,
    );
  }
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

test("owner-only-access policy accepts local Welcome teammates", () => {
  const teammate = makeAgent({
    respondTo: "owner-only",
    respondToAllowlist: [],
  });
  assert.equal(welcomeTeammateHasExpectedAccess(teammate, PUB_B, true), true);
  assert.equal(welcomeTeammateHasExpectedAccess(teammate, PUB_B, false), false);
});

test("access remediation converges for an upgraded owner-only install", () => {
  // Pre-existing installs allowlisted the lead. An owner-only build must move
  // them to owner-only, and the write it makes must satisfy the predicate, so
  // the next provisioning pass makes no further write.
  const allowlisted = makeAgent({
    respondTo: "allowlist",
    respondToAllowlist: [PUB_B],
  });
  const update = welcomeTeammateAccessUpdate(allowlisted, PUB_B, true);
  assert.deepEqual(update, {
    pubkey: PUB_A,
    respondTo: "owner-only",
    respondToAllowlist: [],
  });
  const remediated = makeAgent({
    respondTo: update.respondTo,
    respondToAllowlist: update.respondToAllowlist,
  });
  assert.equal(welcomeTeammateHasExpectedAccess(remediated, PUB_B, true), true);
  assert.equal(welcomeTeammateAccessUpdate(remediated, PUB_B, true), null);
});

test("access remediation allowlists the lead when the build is not owner-only", () => {
  const ownerOnly = makeAgent({
    respondTo: "owner-only",
    respondToAllowlist: [],
  });
  const update = welcomeTeammateAccessUpdate(ownerOnly, PUB_B, false);
  assert.deepEqual(update, {
    pubkey: PUB_A,
    respondTo: "allowlist",
    respondToAllowlist: [PUB_B],
  });
  const remediated = makeAgent({
    respondTo: update.respondTo,
    respondToAllowlist: update.respondToAllowlist,
  });
  assert.equal(
    welcomeTeammateHasExpectedAccess(remediated, PUB_B, false),
    true,
  );
  assert.equal(welcomeTeammateAccessUpdate(remediated, PUB_B, false), null);
});

test("access remediation skips a teammate that already allows the lead", () => {
  const allowlisted = makeAgent({
    respondTo: "allowlist",
    respondToAllowlist: [PUB_B, PUB_C],
  });
  assert.equal(welcomeTeammateAccessUpdate(allowlisted, PUB_B, false), null);
});

test("owner-only-access policy accepts provider Welcome teammates", () => {
  const teammate = makeAgent({
    backend: { type: "provider", id: "remote", config: {} },
    respondTo: "owner-only",
    respondToAllowlist: [],
  });
  assert.equal(welcomeTeammateHasExpectedAccess(teammate, PUB_B, true), true);
  assert.equal(welcomeTeammateHasExpectedAccess(teammate, PUB_B, false), false);
});
