import assert from "node:assert/strict";
import test from "node:test";

import {
  areWelcomeTeammatesOnline,
  buildWelcomeKickoffCloser,
  buildWelcomeKickoffOpener,
  createWelcomeKickoffCoordinator,
  resolveWelcomeAgentSet,
  waitForWelcomeKickoffBeat,
  waitForWelcomeTeammatesOnline,
  welcomeTeammateNeedsRestart,
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

test("teammates are not ready until every harness publishes online presence", () => {
  assert.equal(areWelcomeTeammatesOnline([honey, bumble], undefined), false);
  assert.equal(
    areWelcomeTeammatesOnline([honey, bumble], {
      [honey.pubkey]: "online",
      [bumble.pubkey]: "offline",
    }),
    false,
  );
  assert.equal(
    areWelcomeTeammatesOnline([honey, bumble], {
      [honey.pubkey]: "online",
      [bumble.pubkey]: "online",
    }),
    true,
  );
});

test("readiness wait observes agents becoming online without navigation", async () => {
  let reads = 0;
  const ready = await waitForWelcomeTeammatesOnline([honey, bumble], {
    isCancelled: () => false,
    loadPresence: async () => {
      reads += 1;
      return reads < 3
        ? { [honey.pubkey]: "online", [bumble.pubkey]: "offline" }
        : { [honey.pubkey]: "online", [bumble.pubkey]: "online" };
    },
    pollMs: 0,
    waitMs: 1_000,
  });

  assert.equal(ready, true);
  assert.equal(reads, 3);
});

test("readiness wait cancels when Welcome loses focus", async () => {
  const ready = await waitForWelcomeTeammatesOnline([honey, bumble], {
    isCancelled: () => true,
    loadPresence: async () => {
      throw new Error("cancelled waits must not query");
    },
    pollMs: 0,
    waitMs: 1_000,
  });

  assert.equal(ready, false);
});

test("kickoff beat waits for the configured pacing interval", async () => {
  const startedAt = Date.now();
  assert.equal(await waitForWelcomeKickoffBeat({ waitMs: 10 }), true);
  assert.ok(Date.now() - startedAt >= 8);
});

test("kickoff beat cancels when Welcome loses focus", async () => {
  const controller = new AbortController();
  const beat = waitForWelcomeKickoffBeat({
    signal: controller.signal,
    waitMs: 1_000,
  });
  controller.abort();
  assert.equal(await beat, false);
});

test("kickoff coordinator preserves one task across rerenders and cancels on navigation", () => {
  const coordinator = createWelcomeKickoffCoordinator();
  const first = coordinator.begin("welcome");
  assert.ok(first);
  assert.equal(coordinator.begin("welcome"), null);
  assert.equal(first.signal.aborted, false);

  coordinator.cancel("welcome");
  assert.equal(first.signal.aborted, true);
  assert.ok(coordinator.begin("welcome"));
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

test("closer names teammates that did not reply before the intro wait", () => {
  assert.match(
    buildWelcomeKickoffCloser([], ["Bumble"]),
    /Bumble is taking longer to reply/,
  );
  assert.match(
    buildWelcomeKickoffCloser(["Honey"], ["Bumble"]),
    /Honey and Bumble are taking longer than expected/,
  );
});

test("running teammates restart when their allowlist does not include the lead", () => {
  assert.equal(
    welcomeTeammateNeedsRestart(
      {
        ...honey,
        status: "running",
        respondTo: "allowlist",
        respondToAllowlist: [fizz.pubkey],
      },
      fizz.pubkey,
    ),
    false,
  );
  assert.equal(
    welcomeTeammateNeedsRestart(
      {
        ...bumble,
        status: "running",
        respondTo: "allowlist",
        respondToAllowlist: [honey.pubkey],
      },
      fizz.pubkey,
    ),
    true,
  );
});
