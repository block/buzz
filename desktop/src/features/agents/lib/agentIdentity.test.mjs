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

test("a name is folded to NFC, so one fleet does not split on encoding", () => {
  // macOS input methods and file systems commonly emit NFD, Windows emits NFC.
  // The same name typed on two machines must land on one card.
  const precomposed = "José"; // é as U+00E9
  const decomposed = "José"; // e + U+0301 combining acute

  assert.notEqual(precomposed, decomposed, "the inputs really do differ");
  assert.equal(
    agentDisplayGroupKey({ personaId: "builtin:fizz", name: precomposed }),
    agentDisplayGroupKey({ personaId: "builtin:fizz", name: decomposed }),
  );

  const groups = groupAgentsForDisplay([
    { pubkey: PUBKEY_A, name: precomposed, personaId: "builtin:fizz" },
    { pubkey: PUBKEY_B, name: decomposed, personaId: "builtin:fizz" },
  ]);

  assert.equal(
    groups.length,
    1,
    "two encodings of one name must not render two identical-looking cards",
  );
  assert.equal(groups[0].agents.length, 2);
});

test("the group key cannot be forged by a name containing a separator", () => {
  // Segments are length-prefixed. With a plain `|` join both of these render
  // `persona:a|name:x|name:y`, silently merging two different agents onto one
  // card and leaving one of them unopenable.
  assert.notEqual(
    agentDisplayGroupKey({ personaId: "a", name: "x|name:y" }),
    agentDisplayGroupKey({ personaId: "a|name:x", name: "y" }),
  );
  assert.notEqual(
    agentDisplayGroupKey({ personaId: "builtin:fizz", name: "a:b" }),
    agentDisplayGroupKey({ personaId: "builtin:fizz:a", name: "b" }),
  );
  // A separator in a name is still just a name — same input, same key.
  assert.equal(
    agentDisplayGroupKey({ personaId: "a", name: "x|name:y" }),
    agentDisplayGroupKey({ personaId: "a", name: " X|NAME:Y " }),
  );
});

test("unnamed instances of one persona share a card — documented, not accidental", () => {
  // "", "   ", null and undefined all fold to the same empty name, so several
  // unnamed instances of one persona collapse onto the persona's card. They
  // stay reachable through that card's profile panel, which lists every
  // instance behind it. Asserted so a future change to the fold has to decide
  // this deliberately rather than discover it.
  const groups = groupAgentsForDisplay([
    { pubkey: PUBKEY_A, name: "", personaId: "builtin:fizz" },
    { pubkey: PUBKEY_B, name: "   ", personaId: "builtin:fizz" },
    { pubkey: "c".repeat(64), name: null, personaId: "builtin:fizz" },
    { pubkey: "d".repeat(64), name: "Fizz", personaId: "builtin:fizz" },
  ]);

  assert.deepEqual(
    groups.map((group) => group.name),
    ["", "Fizz"],
  );
  assert.equal(groups[0].agents.length, 3, "no unnamed instance is dropped");
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
