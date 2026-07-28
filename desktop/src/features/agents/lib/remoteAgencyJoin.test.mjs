import assert from "node:assert/strict";
import test from "node:test";

import {
  bindingFromRemoteAgencyProxies,
  buildRemoteAgencyManagedAgentInput,
  findRemoteAgencyBinding,
  findRemoteAgencyProxy,
} from "./remoteAgencyJoin.ts";

const descriptor = {
  sourceUrl: "https://example.com/.well-known/agency.json",
  agencyId: "agency.example",
  name: "Example Agency",
  description: null,
  protocols: ["a2a"],
  capabilities: [],
  agents: [],
  spaces: [],
};

test("builds the reviewed Remote Agency adapter request without secrets", () => {
  const input = buildRemoteAgencyManagedAgentInput(
    descriptor,
    {
      id: "agent-1",
      name: "Scout",
      description: null,
      recordUrl: "https://example.com/agents/scout.json",
      recordRevision: "r1",
      a2aEndpoint: "https://example.com/a2a/scout",
      agentCardUrl: "https://example.com/a2a/card.json",
      capabilities: ["research"],
    },
    "channel-1",
    "space-1",
  );
  assert.deepEqual(input.agentArgs, []);
  assert.equal(input.envVars.BUZZ_A2A_BEARER_TOKEN, undefined);
  assert.equal(
    input.envVars.BUZZ_A2A_BEARER_ENDPOINT,
    "https://example.com/a2a/scout",
  );
  assert.equal(input.envVars.BUZZ_A2A_CHANNEL_REF, "channel-1");
  assert.equal(input.name, "Scout");
  assert.equal(input.parallelism, 1);
  assert.equal(input.startOnAppLaunch, true);
});

test("refuses a participant without a reviewed record or endpoint", () => {
  assert.throws(() =>
    buildRemoteAgencyManagedAgentInput(
      descriptor,
      {
        id: "agent-1",
        name: "Scout",
        description: null,
        recordUrl: null,
        recordRevision: null,
        a2aEndpoint: "https://example.com/a2a/scout",
        agentCardUrl: null,
        capabilities: [],
      },
      "channel-1",
      null,
    ),
  );
  assert.throws(() =>
    buildRemoteAgencyManagedAgentInput(
      descriptor,
      {
        id: "agent-1",
        name: "Scout",
        description: null,
        recordUrl: "https://example.com/agents/scout.json",
        recordRevision: null,
        a2aEndpoint: null,
        agentCardUrl: null,
        capabilities: [],
      },
      "channel-1",
      null,
    ),
  );
});

test("reuses a persisted proxy after a partial join failure", () => {
  const proxy = {
    agentId: "agent-1",
    pubkey: "a".repeat(64),
    channelId: "channel-1",
    spaceId: "space-1",
    recordUrl: "https://example.com/agents/scout.json",
    recordRevision: "r1",
  };
  const binding = bindingFromRemoteAgencyProxies(descriptor, [proxy], "joined");
  assert.equal(
    findRemoteAgencyProxy(binding.proxies, "agent-1", "channel-1", "space-1"),
    proxy,
  );
  assert.deepEqual(binding.agentIds, ["agent-1"]);
  assert.deepEqual(binding.spaceIds, ["space-1"]);
  assert.deepEqual(binding.channelIds, ["channel-1"]);
  assert.equal(binding.joinedAt, "joined");
});

test("matches a persisted Agency binding across local loopback aliases", () => {
  const binding = {
    ...bindingFromRemoteAgencyProxies(descriptor, [], "joined"),
    sourceUrl: "http://localhost:1337/.well-known/agency.json",
    agencyId: "agency.local",
  };
  const localDescriptor = {
    ...descriptor,
    sourceUrl: "http://127.0.0.1:1337/.well-known/agency.json",
    agencyId: "agency.local",
  };
  assert.equal(findRemoteAgencyBinding([binding], localDescriptor), binding);
});

test("does not migrate a binding across public hosts or Agency identities", () => {
  const binding = bindingFromRemoteAgencyProxies(descriptor, [], "joined");
  assert.equal(
    findRemoteAgencyBinding([binding], {
      ...descriptor,
      sourceUrl: "https://other.example/.well-known/agency.json",
    }),
    undefined,
  );
  assert.equal(
    findRemoteAgencyBinding(
      [
        {
          ...binding,
          sourceUrl: "http://localhost:1337/.well-known/agency.json",
        },
      ],
      {
        ...descriptor,
        sourceUrl: "http://127.0.0.1:1337/.well-known/agency.json",
        agencyId: "other-agency",
      },
    ),
    undefined,
  );
});
