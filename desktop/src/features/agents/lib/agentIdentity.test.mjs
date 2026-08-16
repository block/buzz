import assert from "node:assert/strict";
import test from "node:test";

import {
  agentDisplayGroupKey,
  agentIdentityKey,
  groupAgentsForDisplay,
} from "./agentIdentity.ts";

const PUBKEY_A = "a".repeat(64);
const PUBKEY_B = "b".repeat(64);

test("agent identity is the pubkey, never persona metadata or a name", () => {
  const left = {
    pubkey: PUBKEY_A,
    name: "Bumble",
    personaId: "builtin:bumble",
  };
  const right = {
    pubkey: PUBKEY_B,
    name: "Bumble",
    personaId: "builtin:bumble",
  };

  assert.notEqual(agentIdentityKey(left), agentIdentityKey(right));
  assert.equal(
    agentIdentityKey({ pubkey: ` ${PUBKEY_A.toUpperCase()} ` }),
    agentIdentityKey({ pubkey: PUBKEY_A, personaId: "something-else" }),
  );
  assert.equal(agentIdentityKey({ pubkey: null }), null);
  assert.equal(agentIdentityKey({}), null);
});

test("the display group key separates renamed instances of one persona", () => {
  const claude = {
    pubkey: PUBKEY_A,
    name: "Claude",
    personaId: "builtin:fizz",
  };
  const fizz = { pubkey: PUBKEY_B, name: "Fizz", personaId: "builtin:fizz" };

  assert.notEqual(agentDisplayGroupKey(claude), agentDisplayGroupKey(fizz));
  assert.equal(
    agentDisplayGroupKey(claude),
    agentDisplayGroupKey({ ...claude, pubkey: PUBKEY_B, name: " claude " }),
  );
  assert.notEqual(
    agentDisplayGroupKey(claude),
    agentDisplayGroupKey({ ...claude, personaId: "builtin:honey" }),
  );
});

test("display grouping keeps every distinct identity and drops repeats", () => {
  const agents = [
    { pubkey: PUBKEY_A, name: "Claude", personaId: "builtin:fizz" },
    { pubkey: PUBKEY_B, name: "Fizz", personaId: "builtin:fizz" },
    { pubkey: PUBKEY_A, name: "Claude", personaId: "builtin:fizz" },
  ];

  const groups = groupAgentsForDisplay(agents);

  assert.deepEqual(
    groups.map((group) => group.name),
    ["Claude", "Fizz"],
  );
  assert.deepEqual(
    new Set(groups.flatMap((group) => group.agents).map(agentIdentityKey)),
    new Set([
      agentIdentityKey({ pubkey: PUBKEY_A }),
      agentIdentityKey(agents[1]),
    ]),
  );
  assert.equal(groups[0].agents.length, 1);
});
