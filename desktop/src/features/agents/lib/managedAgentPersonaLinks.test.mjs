import assert from "node:assert/strict";
import test from "node:test";

import {
  buildManagedAgentPersonaLinks,
  findRemotePersonaAgent,
} from "./managedAgentPersonaLinks.ts";

test("remote kind:30177 links participate in persona identity dedup", () => {
  const remote = {
    pubkey: "A".repeat(64),
    name: "Rimac-Buzz",
    personaId: "rimac-definition",
  };
  const links = buildManagedAgentPersonaLinks([], [remote]);

  assert.equal(
    links.byPubkey.get(remote.pubkey.toLowerCase()),
    "rimac-definition",
  );
  assert.deepEqual(links.personaIds, new Set(["rimac-definition"]));
});

test("remote persona body blocks a second-install mint but local body does not", () => {
  const reference = {
    pubkey: "a".repeat(64),
    name: "Rimac-Buzz",
    personaId: "rimac-definition",
  };

  assert.equal(
    findRemotePersonaAgent("rimac-definition", new Set(), [reference]),
    reference,
  );
  assert.equal(
    findRemotePersonaAgent(
      "rimac-definition",
      new Set([reference.pubkey.toUpperCase()]),
      [reference],
    ),
    undefined,
  );
});
