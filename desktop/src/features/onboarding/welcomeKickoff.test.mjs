import assert from "node:assert/strict";
import test from "node:test";

import {
  areWelcomeTeammatesOnline,
  buildWelcomeKickoffCloser,
  buildWelcomeKickoffOpener,
  buildWelcomeKickoffOpenerSendInput,
  classifyWelcomeKickoffResolution,
  createWelcomeKickoffCoordinator,
  mergeKickoffEvents,
  resolveWelcomeAgentSet,
  selectWelcomeKickoffIntroTeammates,
  waitForWelcomeKickoffBeat,
  waitForWelcomeTeammatesOnline,
  welcomeTeammateNeedsRestart,
} from "./welcomeKickoff.ts";

function agent(name, personaId, pubkey) {
  return {
    name,
    personaId,
    teamId: "builtin-team:welcome",
    pubkey,
    relayUrl: "ws://localhost:3000",
    status: "stopped",
    lastError: null,
    lastStartedAt: null,
  };
}

const diego = agent("Diego", "builtin:diego", "f".repeat(64));
const murietta = agent("Murietta", "builtin:murietta", "h".repeat(64));
const montero = agent("Montero", "builtin:montero", "b".repeat(64));

test("resolveWelcomeAgentSet orders agents by stable persona identity", () => {
  assert.deepEqual(resolveWelcomeAgentSet([montero, diego, murietta]), {
    lead: diego,
    teammates: [murietta, montero],
  });
  assert.equal(resolveWelcomeAgentSet([diego, murietta]), null);
});

test("opener uses current agent names and requests bounded simultaneous intros", () => {
  const opener = buildWelcomeKickoffOpener({ ...diego, name: "Diego" }, [
    { ...murietta, name: "Murietta" },
    montero,
  ]);

  assert.match(opener, /I’m Diego, the seasoned project manager/);
  assert.match(opener, /@Murietta and @Montero/);
  assert.doesNotMatch(opener, /@@/);
  assert.match(opener, /sentence or two/);
  assert.match(opener, /Don't start any work yet/);
});

test("teammates are not ready until every harness publishes online presence", () => {
  assert.equal(
    areWelcomeTeammatesOnline([murietta, montero], undefined),
    false,
  );
  assert.equal(
    areWelcomeTeammatesOnline([murietta, montero], {
      [murietta.pubkey]: "online",
      [montero.pubkey]: "offline",
    }),
    false,
  );
  assert.equal(
    areWelcomeTeammatesOnline([murietta, montero], {
      [murietta.pubkey]: "online",
      [montero.pubkey]: "online",
    }),
    true,
  );
});

test("readiness wait observes agents becoming online without navigation", async () => {
  let reads = 0;
  const ready = await waitForWelcomeTeammatesOnline([murietta, montero], {
    isCancelled: () => false,
    loadPresence: async () => {
      reads += 1;
      return reads < 3
        ? { [murietta.pubkey]: "online", [montero.pubkey]: "offline" }
        : { [murietta.pubkey]: "online", [montero.pubkey]: "online" };
    },
    pollMs: 0,
    waitMs: 1_000,
  });

  assert.deepEqual(ready, [murietta, montero]);
  assert.equal(reads, 3);
});

test("readiness wait retries transient presence failures", async () => {
  let reads = 0;
  const ready = await waitForWelcomeTeammatesOnline([murietta, montero], {
    isCancelled: () => false,
    loadPresence: async () => {
      reads += 1;
      if (reads === 1) throw new Error("relay unavailable");
      return { [murietta.pubkey]: "online", [montero.pubkey]: "online" };
    },
    pollMs: 0,
    waitMs: 1_000,
  });

  assert.deepEqual(ready, [murietta, montero]);
  assert.equal(reads, 2);
});

test("readiness wait cancels when Welcome loses focus", async () => {
  const ready = await waitForWelcomeTeammatesOnline([murietta, montero], {
    isCancelled: () => true,
    loadPresence: async () => {
      throw new Error("cancelled waits must not query");
    },
    pollMs: 0,
    waitMs: 1_000,
  });

  assert.deepEqual(ready, []);
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
  assert.match(
    buildWelcomeKickoffCloser(["Murietta"]),
    /Murietta is having trouble/,
  );
  assert.match(
    buildWelcomeKickoffCloser(["Murietta", "Montero"]),
    /Murietta and Montero couldn't start/,
  );
  assert.match(
    buildWelcomeKickoffCloser(["Murietta", "Montero"]),
    /I'm still here to help/,
  );
});

test("closer names teammates that did not reply before the intro wait", () => {
  assert.match(
    buildWelcomeKickoffCloser([], ["Montero"]),
    /Montero is taking longer to reply/,
  );
  assert.match(
    buildWelcomeKickoffCloser(["Murietta"], ["Montero"]),
    /Murietta and Montero are taking longer than expected/,
  );
});

test("running teammates restart when their allowlist does not include the lead", () => {
  assert.equal(
    welcomeTeammateNeedsRestart(
      {
        ...murietta,
        status: "running",
        respondTo: "allowlist",
        respondToAllowlist: [diego.pubkey],
      },
      diego.pubkey,
    ),
    false,
  );
  assert.equal(
    welcomeTeammateNeedsRestart(
      {
        ...montero,
        status: "running",
        respondTo: "allowlist",
        respondToAllowlist: [murietta.pubkey],
      },
      diego.pubkey,
    ),
    true,
  );
});

test("opener keeps partial-readiness warm and mentions only online teammates", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const introTeammates = selectWelcomeKickoffIntroTeammates(
    agentSet.teammates,
    [murietta],
  );
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    introTeammates,
    "welcome-1",
  );

  assert.deepEqual(input.mentionPubkeys, [murietta.pubkey]);
  assert.deepEqual(input.additionalMarkers, []);
  assert.match(input.content, /@Murietta, introduce yourself/);
  assert.doesNotMatch(input.content, /@@/);
  assert.doesNotMatch(
    input.content,
    /Montero.*trouble|couldn't start|taking longer/i,
  );
});

test("opener introduces Diego and tags the owner's pubkey", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const owner = { pubkey: "owner-pubkey-hex", displayName: "Morgan" };
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    agentSet.teammates,
    "welcome-1",
    owner,
  );

  assert.deepEqual(input.mentionPubkeys, [
    murietta.pubkey,
    montero.pubkey,
    owner.pubkey,
  ]);
  assert.match(
    input.content,
    /^Welcome to Zorro, your interface between your employees and your specialized AI agents\. I’m Diego, the seasoned project manager\./,
  );
  // The raw pubkey must never leak into the visible copy.
  assert.doesNotMatch(input.content, /owner-pubkey-hex/);
});

test("opener keeps the workspace greeting when the display name is missing", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const owner = { pubkey: "owner-pubkey-hex", displayName: "  " };
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    agentSet.teammates,
    "welcome-1",
    owner,
  );

  // Still tagged for the Inbox mentions feed, with no visible owner name.
  assert.ok(input.mentionPubkeys.includes(owner.pubkey));
  assert.match(input.content, /^Welcome to Zorro/);
  assert.doesNotMatch(input.content, /@\s/);
});

test("opener introduces Diego and tags the owner even when no teammates come online", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const input = buildWelcomeKickoffOpenerSendInput(agentSet, [], "welcome-1", {
    pubkey: "owner-pubkey-hex",
    displayName: "Morgan",
  });

  assert.deepEqual(input.mentionPubkeys, ["owner-pubkey-hex"]);
  assert.equal(input.additionalMarkers.length, 1);
  assert.match(input.content, /^Welcome to Zorro/);
});

test("opener does not duplicate the owner pubkey if already mentioned", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    [murietta],
    "welcome-1",
    { pubkey: murietta.pubkey, displayName: murietta.name },
  );

  assert.deepEqual(input.mentionPubkeys, [murietta.pubkey]);
});

test("opener degrades to one seeded Diego message when no teammate comes online", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const input = buildWelcomeKickoffOpenerSendInput(agentSet, [], "welcome-1");

  assert.deepEqual(input.mentionPubkeys, []);
  assert.equal(input.additionalMarkers.length, 1);
  assert.match(input.content, /I’m Diego, the seasoned project manager/);
  assert.match(input.content, /What can we help you build/);
  assert.doesNotMatch(
    input.content,
    /introduce yourselves|trouble|couldn't start|taking longer/i,
  );
});

test("readiness wait returns the subset that became online by the deadline", async () => {
  const online = await waitForWelcomeTeammatesOnline([murietta, montero], {
    isCancelled: () => false,
    loadPresence: async () => ({
      [murietta.pubkey]: "online",
      [montero.pubkey]: "offline",
    }),
    pollMs: 0,
    waitMs: 0,
  });

  assert.deepEqual(online, [murietta]);
});

function relayEvent({ id, pubkey, createdAt = 1, tags = [], content = "" }) {
  return {
    id,
    pubkey,
    created_at: createdAt,
    kind: 9,
    tags,
    content,
    sig: "sig",
  };
}

test("closer classification sees replies that arrive during the final beat", async () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const opener = relayEvent({
    id: "opener",
    pubkey: diego.pubkey,
    tags: [["client", "buzz-welcome-kickoff.opener.v1"]],
  });
  const events = [opener];

  const beforeBeat = classifyWelcomeKickoffResolution(events, opener, agentSet);
  assert.deepEqual(
    beforeBeat.unresolved.map((agent) => agent.name),
    ["Murietta", "Montero"],
  );

  const beat = waitForWelcomeKickoffBeat({ waitMs: 5 });
  events.push(
    relayEvent({
      id: "murietta-intro",
      pubkey: murietta.pubkey,
      createdAt: 2,
      tags: [
        ["e", opener.id, "", "root"],
        ["e", opener.id, "", "reply"],
      ],
    }),
  );
  assert.equal(await beat, true);

  const afterBeat = classifyWelcomeKickoffResolution(events, opener, agentSet);
  assert.deepEqual(
    afterBeat.unresolved.map((agent) => agent.name),
    ["Montero"],
  );
});

function introReply(id, pubkey, openerId) {
  return relayEvent({
    id,
    pubkey,
    createdAt: 2,
    tags: [
      ["e", openerId, "", "root"],
      ["e", openerId, "", "reply"],
    ],
  });
}

const kickoffOpener = relayEvent({
  id: "opener",
  pubkey: diego.pubkey,
  tags: [["client", "buzz-welcome-kickoff.opener.v1"]],
});

// The bug this branch fixes: teammate intros are thread replies, which the
// channel window excludes from the main timeline. So the kickoff saw the
// opener and never the intros, and the closer stalled until the user happened
// to click into the thread. Merging the opener's subtree in is the fix.
test("intro replies reach the closer classification without the user opening the thread", () => {
  const agentSet = { lead: diego, teammates: [murietta, montero] };
  const channelEvents = [kickoffOpener];
  const openerReplies = [
    introReply("murietta-intro", murietta.pubkey, kickoffOpener.id),
    introReply("montero-intro", montero.pubkey, kickoffOpener.id),
  ];

  // Pin the pre-fix behaviour: on the channel events alone, both teammates
  // look silent forever. This is what stalled the closer.
  assert.deepEqual(
    classifyWelcomeKickoffResolution(
      channelEvents,
      kickoffOpener,
      agentSet,
    ).unresolved.map((agent) => agent.name),
    ["Murietta", "Montero"],
  );

  // With the subtree merged in, the same intros resolve the kickoff.
  assert.deepEqual(
    classifyWelcomeKickoffResolution(
      mergeKickoffEvents(channelEvents, openerReplies),
      kickoffOpener,
      agentSet,
    ).unresolved,
    [],
  );
});

test("merging the opener subtree never double-counts an already-visible reply", () => {
  const muriettaIntro = introReply(
    "murietta-intro",
    murietta.pubkey,
    kickoffOpener.id,
  );
  // An open thread feeds the same replies in through both sources.
  const merged = mergeKickoffEvents(
    [kickoffOpener, muriettaIntro],
    [
      muriettaIntro,
      introReply("montero-intro", montero.pubkey, kickoffOpener.id),
    ],
  );

  assert.deepEqual(
    merged.map((event) => event.id),
    ["opener", "murietta-intro", "montero-intro"],
  );
});

test("merging with no subtree replies leaves the channel events untouched", () => {
  const channelEvents = [kickoffOpener];
  assert.equal(mergeKickoffEvents(channelEvents, []), channelEvents);
});
