import assert from "node:assert/strict";
import test from "node:test";

import {
  BUZZ_HERMES_OK_PREFIX,
  RapidPostSaveRouteError,
  assertRapidPostSaveRoute,
  buildRapidTestPrompt,
  createSmokeId,
  filterEligibleRapidTestChannels,
  pickDefaultRapidTestChannelId,
  revalidateRapidPostSaveRoute,
  runRapidAgentPostSaveAction,
} from "./agentRapidTest.ts";

const PUB_A = "a".repeat(64);
const PUB_B = "b".repeat(64);

function makeChannel(overrides = {}) {
  return {
    id: "channel-1",
    name: "general",
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("filterEligibleRapidTestChannels keeps joined, non-archived channels that contain the agent (DMs allowed)", () => {
  const dmChannel = makeChannel({
    id: "dm-1",
    name: "agent-chat",
    channelType: "dm",
    participantPubkeys: [PUB_A, PUB_B],
  });
  const streamChannel = makeChannel({
    id: "stream-1",
    name: "ops",
    channelType: "stream",
    participantPubkeys: [PUB_A, PUB_B],
  });
  const forumChannel = makeChannel({
    id: "forum-1",
    name: "announcements",
    channelType: "forum",
    participantPubkeys: [PUB_A, PUB_B],
  });
  const archivedChannel = makeChannel({
    id: "arch-1",
    name: "old",
    channelType: "stream",
    archivedAt: "2024-01-01T00:00:00Z",
    participantPubkeys: [PUB_A, PUB_B],
  });
  const notJoinedChannel = makeChannel({
    id: "leave-1",
    name: "left",
    channelType: "stream",
    isMember: false,
    participantPubkeys: [PUB_A, PUB_B],
  });
  const agentMissingChannel = makeChannel({
    id: "no-agent-1",
    name: "solo",
    channelType: "stream",
    participantPubkeys: [PUB_B],
  });

  const result = filterEligibleRapidTestChannels(
    [
      dmChannel,
      streamChannel,
      forumChannel,
      archivedChannel,
      notJoinedChannel,
      agentMissingChannel,
    ],
    { pubkey: PUB_A },
  );

  assert.deepEqual(
    result.map((c) => c.id),
    ["dm-1", "stream-1"],
  );
});

test("filterEligibleRapidTestChannels is case-insensitive and tolerates whitespace in pubkeys", () => {
  const channel = makeChannel({
    participantPubkeys: [`  ${PUB_A.toUpperCase()}  `],
  });

  const result = filterEligibleRapidTestChannels([channel], { pubkey: PUB_A });

  assert.equal(result.length, 1);
  assert.equal(result[0].id, channel.id);
});

test("filterEligibleRapidTestChannels falls back to memberPubkeys when participantPubkeys is absent", () => {
  const channel = makeChannel({
    id: "legacy-1",
    memberPubkeys: [PUB_A, PUB_B],
    participantPubkeys: [],
  });

  const result = filterEligibleRapidTestChannels([channel], { pubkey: PUB_A });

  assert.equal(result.length, 1);
  assert.equal(result[0].id, "legacy-1");
});

test("filterEligibleRapidTestChannels returns empty when channels or agent missing", () => {
  assert.deepEqual(
    filterEligibleRapidTestChannels(undefined, { pubkey: PUB_A }),
    [],
  );
  assert.deepEqual(filterEligibleRapidTestChannels([makeChannel()], null), []);
  assert.deepEqual(
    filterEligibleRapidTestChannels([makeChannel()], undefined),
    [],
  );
});

test("pickDefaultRapidTestChannelId returns null when nothing eligible", () => {
  assert.equal(pickDefaultRapidTestChannelId([], "any"), null);
  assert.equal(pickDefaultRapidTestChannelId([], null), null);
});

test("pickDefaultRapidTestChannelId preserves a still-eligible selection", () => {
  const channels = [makeChannel({ id: "a" }), makeChannel({ id: "b" })];

  assert.equal(pickDefaultRapidTestChannelId(channels, "b"), "b");
});

test("pickDefaultRapidTestChannelId falls back to first eligible when current is invalid", () => {
  const channels = [makeChannel({ id: "a" }), makeChannel({ id: "b" })];

  assert.equal(pickDefaultRapidTestChannelId(channels, "missing"), "a");
  assert.equal(pickDefaultRapidTestChannelId(channels, null), "a");
});

test("createSmokeId is unique, short, and safe to place after the reply prefix", () => {
  const ids = new Set();
  for (let i = 0; i < 25; i += 1) {
    ids.add(createSmokeId());
  }
  assert.equal(ids.size, 25, "smoke ids must be unique across 25 draws");

  for (const id of ids) {
    assert.match(id, /^[0-9a-z]{8}$/);
    assert.ok(!id.includes(BUZZ_HERMES_OK_PREFIX), id);
  }
});

test("createSmokeId accepts an injected random source for deterministic tests", () => {
  const fake = {
    randomValues: (buffer) => {
      for (let i = 0; i < buffer.length; i += 1) {
        buffer[i] = i;
      }
      return buffer;
    },
  };

  const a = createSmokeId(fake);
  const b = createSmokeId(fake);

  assert.equal(a, b);
  assert.equal(a, "01234567");
});

test("buildRapidTestPrompt is deterministic for the same inputs and labels the smoke id", () => {
  const at = new Date("2025-01-01T00:00:00.000Z");
  const a = buildRapidTestPrompt({ smokeId: "token_1", generatedAt: at });
  const b = buildRapidTestPrompt({ smokeId: "token_1", generatedAt: at });

  assert.equal(a.body, b.body);
  assert.match(a.body, /\[buzz-hermes-smoke\]/);
  assert.match(a.body, /BUZZ_HERMES_OK token_1/);
  assert.match(a.body, /smoke id: token_1/);
  assert.match(a.body, /generated at: 2025-01-01T00:00:00Z/);
  assert.ok(
    !a.body.toLowerCase().includes("nsec") &&
      !a.body.toLowerCase().includes("api_key") &&
      !a.body.toLowerCase().includes("password"),
    "must not embed secret markers",
  );
});

test("buildRapidTestPrompt falls back to 'unknown' for invalid dates", () => {
  const prompt = buildRapidTestPrompt({
    smokeId: "token_x",
    generatedAt: "not-a-date",
  });

  assert.match(prompt.body, /generated at: unknown/);
});

const ROUTE_AGENT = {
  runtime: "hermes-buzz-mcp",
  agentCommand: "C:\\tools\\hermes-acp.exe",
  acpCommand: "C:\\tools\\buzz-acp.exe",
};

function assertRoute(overrides = {}) {
  return assertRapidPostSaveRoute({
    savedAgent: ROUTE_AGENT,
    savedRuntimeId: "hermes-buzz-mcp",
    expectedRuntimeId: "hermes-buzz-mcp",
    expectedAgentCommand: "C:/tools/hermes-acp.exe",
    expectedAcpCommand: "C:/tools/buzz-acp.exe",
    catalogRuntimeCommand: "C:/tools/hermes-acp.exe",
    ...overrides,
  });
}

test("assertRapidPostSaveRoute accepts the persisted selected route", () => {
  assert.doesNotThrow(() => assertRoute());
});

test("assertRapidPostSaveRoute rejects a persisted runtime drift", () => {
  assert.throws(
    () => assertRoute({ savedRuntimeId: "codex" }),
    /persisted runtime changed/,
  );
});

test("assertRapidPostSaveRoute accepts a caller-resolved inherited runtime", () => {
  assert.doesNotThrow(() => assertRoute({ savedRuntimeId: "hermes-buzz-mcp" }));
});

test("assertRapidPostSaveRoute accepts an explicit custom command without a catalog entry", () => {
  assert.doesNotThrow(() =>
    assertRoute({
      savedRuntimeId: "custom",
      expectedRuntimeId: "custom",
      catalogRuntimeCommand: null,
      catalogRequired: false,
    }),
  );
});

test("assertRapidPostSaveRoute throws the typed fail-closed error", () => {
  assert.throws(
    () => assertRoute({ savedRuntimeId: "codex" }),
    RapidPostSaveRouteError,
  );
});

test("revalidateRapidPostSaveRoute resolves an inherited runtime from a fresh persona", async () => {
  await assert.doesNotReject(() =>
    revalidateRapidPostSaveRoute({
      savedAgent: { ...ROUTE_AGENT, runtime: null, personaId: "persona-1" },
      expectedRuntimeId: "hermes-buzz-mcp",
      expectedAgentCommand: "C:/tools/hermes-acp.exe",
      expectedAcpCommand: "C:/tools/buzz-acp.exe",
      refetchRuntimes: async () => ({
        data: [{ id: "hermes-buzz-mcp", command: "C:/tools/hermes-acp.exe" }],
        isError: false,
      }),
      refetchPersonas: async () => ({
        data: [{ id: "persona-1", runtime: "hermes-buzz-mcp" }],
        isError: false,
      }),
    }),
  );
});

test("revalidateRapidPostSaveRoute fails closed when catalog refresh fails", async () => {
  await assert.rejects(
    () =>
      revalidateRapidPostSaveRoute({
        savedAgent: {
          ...ROUTE_AGENT,
          runtime: "hermes-buzz-mcp",
          personaId: null,
        },
        expectedRuntimeId: "hermes-buzz-mcp",
        expectedAgentCommand: "C:/tools/hermes-acp.exe",
        expectedAcpCommand: "C:/tools/buzz-acp.exe",
        refetchRuntimes: async () => ({ data: undefined, isError: true }),
        refetchPersonas: async () => ({ data: undefined, isError: false }),
      }),
    RapidPostSaveRouteError,
  );
});

test("assertRapidPostSaveRoute rejects a missing catalog route", () => {
  assert.throws(
    () => assertRoute({ catalogRuntimeCommand: null }),
    /saved harness route changed/,
  );
});

test("assertRapidPostSaveRoute rejects saved harness command drift", () => {
  assert.throws(
    () =>
      assertRoute({
        savedAgent: { ...ROUTE_AGENT, agentCommand: "codex-acp.exe" },
      }),
    /saved harness route changed/,
  );
});

test("assertRapidPostSaveRoute rejects catalog harness command drift", () => {
  assert.throws(
    () => assertRoute({ catalogRuntimeCommand: "codex-acp.exe" }),
    /saved harness route changed/,
  );
});

test("assertRapidPostSaveRoute rejects ACP sidecar drift", () => {
  assert.throws(
    () =>
      assertRoute({
        savedAgent: { ...ROUTE_AGENT, acpCommand: "C:/tools/other-acp.exe" },
      }),
    /saved ACP sidecar changed/,
  );
});

test("runRapidAgentPostSaveAction restarts before owner send and opens the returned root thread", async () => {
  const calls = [];
  const prompt = buildRapidTestPrompt({
    smokeId: "smoke123",
    generatedAt: "2025-01-01T00:00:00Z",
  });
  const channel = makeChannel({ id: "channel-1" });

  const outcome = await runRapidAgentPostSaveAction({
    mode: "smoke",
    pubkey: PUB_A,
    relayUrl: "wss://relay.example",
    selection: { channel, channelId: channel.id, prompt },
    restart: async (pubkey, relayUrl) => {
      calls.push(["restart", pubkey, relayUrl]);
    },
    waitForReady: async () => {
      calls.push(["ready"]);
    },
    sendOwnerMessage: async (targetChannel, content, mentionPubkeys) => {
      calls.push(["send", targetChannel.id, content, mentionPubkeys]);
      return { eventId: "event-1" };
    },
    openThread: async (channelId, eventId) => {
      calls.push(["open", channelId, eventId]);
    },
  });

  assert.equal(outcome.kind, "smoke-posted");
  assert.equal(outcome.threadOpened, true);
  assert.deepEqual(
    calls.map(([name]) => name),
    ["restart", "ready", "send", "open"],
  );
  assert.deepEqual(calls[2][3], [PUB_A]);
  assert.match(calls[2][2], /BUZZ_HERMES_OK smoke123/);
  assert.deepEqual(calls[3], ["open", "channel-1", "event-1"]);
});

test("runRapidAgentPostSaveAction preserves a posted event when thread navigation fails", async () => {
  let sends = 0;
  const channel = makeChannel({ id: "channel-1" });
  const outcome = await runRapidAgentPostSaveAction({
    mode: "smoke",
    pubkey: PUB_A,
    relayUrl: "wss://relay.example",
    selection: {
      channel,
      channelId: channel.id,
      prompt: buildRapidTestPrompt({
        smokeId: "smoke456",
        generatedAt: "2025-01-01T00:00:00Z",
      }),
    },
    restart: async () => {},
    sendOwnerMessage: async () => {
      sends += 1;
      return { eventId: "event-2" };
    },
    openThread: async () => {
      throw new Error("navigation failed");
    },
  });

  assert.equal(sends, 1);
  assert.deepEqual(outcome, {
    kind: "smoke-posted",
    channelId: "channel-1",
    eventId: "event-2",
    threadOpened: false,
  });
});

test("runRapidAgentPostSaveAction does not send for save or restart-only modes", async () => {
  const calls = [];
  const dependencies = {
    pubkey: PUB_A,
    relayUrl: "wss://relay.example",
    selection: null,
    restart: async () => calls.push("restart"),
    sendOwnerMessage: async () => {
      calls.push("send");
      return { eventId: "unexpected" };
    },
    openThread: async () => calls.push("open"),
  };

  assert.equal(
    await runRapidAgentPostSaveAction({ mode: "save", ...dependencies }),
    null,
  );
  assert.deepEqual(calls, []);

  const restarted = await runRapidAgentPostSaveAction({
    mode: "restart",
    ...dependencies,
  });
  assert.equal(restarted.kind, "restarted");
  assert.deepEqual(calls, ["restart"]);
});
