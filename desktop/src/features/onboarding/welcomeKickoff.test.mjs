import assert from "node:assert/strict";
import test from "node:test";

import {
  buildWelcomeKickoffCloser,
  buildWelcomeKickoffOpener,
  resolveWelcomeAgentSet,
} from "./welcomeKickoff.ts";

function agent(name, personaId, pubkey) {
  return {
    name,
    personaId,
    pubkey,
    relayUrl: "ws://localhost:3000",
    status: "stopped",
    lastError: null,
    lastStartedAt: null,
  };
}

const fizz = agent("Fizz", "builtin:fizz", "f".repeat(64));
const honey = agent("Honey", "builtin:honey", "h".repeat(64));
const bumble = agent("Bumble", "builtin:bumble", "b".repeat(64));

test("resolveWelcomeAgentSet orders agents by stable persona identity", () => {
  assert.deepEqual(resolveWelcomeAgentSet([bumble, fizz, honey]), {
    lead: fizz,
    teammates: [honey, bumble],
  });
  assert.equal(resolveWelcomeAgentSet([fizz, honey]), null);
});

test("opener uses current agent names and requests bounded simultaneous intros", () => {
  const opener = buildWelcomeKickoffOpener({ ...fizz, name: "Fizzy" }, [
    { ...honey, name: "Honeybee" },
    bumble,
  ]);

  assert.match(opener, /I'm Fizzy/);
  assert.match(opener, /@Honeybee and @Bumble/);
  assert.match(opener, /sentence or two/);
  assert.match(opener, /Don't start any work yet/);
});

test("closer degrades coherently for partial and total startup failure", () => {
  assert.match(buildWelcomeKickoffCloser([]), /What can we help you build/);
  assert.match(buildWelcomeKickoffCloser(["Honey"]), /Honey is having trouble/);
  assert.match(
    buildWelcomeKickoffCloser(["Honey", "Bumble"]),
    /Honey and Bumble couldn't start/,
  );
  assert.match(
    buildWelcomeKickoffCloser(["Honey", "Bumble"]),
    /I'm still here to help/,
  );
});
