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
  assert.doesNotMatch(opener, /@@/);
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

  assert.deepEqual(ready, [honey, bumble]);
  assert.equal(reads, 3);
});

test("readiness wait retries transient presence failures", async () => {
  let reads = 0;
  const ready = await waitForWelcomeTeammatesOnline([honey, bumble], {
    isCancelled: () => false,
    loadPresence: async () => {
      reads += 1;
      if (reads === 1) throw new Error("relay unavailable");
      return { [honey.pubkey]: "online", [bumble.pubkey]: "online" };
    },
    pollMs: 0,
    waitMs: 1_000,
  });

  assert.deepEqual(ready, [honey, bumble]);
  assert.equal(reads, 2);
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

test("opener keeps partial-readiness warm and mentions only online teammates", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const introTeammates = selectWelcomeKickoffIntroTeammates(
    agentSet.teammates,
    [honey],
  );
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    introTeammates,
    "welcome-1",
  );

  assert.deepEqual(input.mentionPubkeys, [honey.pubkey]);
  assert.deepEqual(input.additionalMarkers, []);
  assert.match(input.content, /@Honey, introduce yourself/);
  assert.doesNotMatch(input.content, /@@/);
  assert.doesNotMatch(
    input.content,
    /Bumble.*trouble|couldn't start|taking longer/i,
  );
});

test("opener greets the owner by name and tags their pubkey", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const owner = { pubkey: "owner-pubkey-hex", displayName: "Morgan" };
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    agentSet.teammates,
    "welcome-1",
    owner,
  );

  assert.deepEqual(input.mentionPubkeys, [
    honey.pubkey,
    bumble.pubkey,
    owner.pubkey,
  ]);
  assert.match(input.content, /^Hi @Morgan, I'm Fizz\./);
  // The raw pubkey must never leak into the visible copy.
  assert.doesNotMatch(input.content, /owner-pubkey-hex/);
});

test("opener falls back to an unnamed greeting when the display name is missing", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const owner = { pubkey: "owner-pubkey-hex", displayName: "  " };
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    agentSet.teammates,
    "welcome-1",
    owner,
  );

  // Still tagged for the Inbox mentions feed, just no visible greeting name.
  assert.ok(input.mentionPubkeys.includes(owner.pubkey));
  assert.match(input.content, /^Hi, I'm Fizz\./);
  assert.doesNotMatch(input.content, /@\s/);
});

test("opener greets and tags the owner even when no teammates come online", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const input = buildWelcomeKickoffOpenerSendInput(agentSet, [], "welcome-1", {
    pubkey: "owner-pubkey-hex",
    displayName: "Morgan",
  });

  assert.deepEqual(input.mentionPubkeys, ["owner-pubkey-hex"]);
  assert.equal(input.additionalMarkers.length, 1);
  assert.match(input.content, /^Hi @Morgan, I'm Fizz\./);
});

test("opener does not duplicate the owner pubkey if already mentioned", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const input = buildWelcomeKickoffOpenerSendInput(
    agentSet,
    [honey],
    "welcome-1",
    { pubkey: honey.pubkey, displayName: honey.name },
  );

  assert.deepEqual(input.mentionPubkeys, [honey.pubkey]);
});

test("opener degrades to one seeded Fizz message when no teammate comes online", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const input = buildWelcomeKickoffOpenerSendInput(agentSet, [], "welcome-1");

  assert.deepEqual(input.mentionPubkeys, []);
  assert.equal(input.additionalMarkers.length, 1);
  assert.match(input.content, /I'm here with Honey and Bumble/);
  assert.match(input.content, /What can we help you build/);
  assert.doesNotMatch(
    input.content,
    /introduce yourselves|trouble|couldn't start|taking longer/i,
  );
});

test("readiness wait returns the subset that became online by the deadline", async () => {
  const online = await waitForWelcomeTeammatesOnline([honey, bumble], {
    isCancelled: () => false,
    loadPresence: async () => ({
      [honey.pubkey]: "online",
      [bumble.pubkey]: "offline",
    }),
    pollMs: 0,
    waitMs: 0,
  });

  assert.deepEqual(online, [honey]);
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
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const opener = relayEvent({
    id: "opener",
    pubkey: fizz.pubkey,
    tags: [["client", "buzz-welcome-kickoff.opener.v1"]],
  });
  const events = [opener];

  const beforeBeat = classifyWelcomeKickoffResolution(events, opener, agentSet);
  assert.deepEqual(
    beforeBeat.unresolved.map((agent) => agent.name),
    ["Honey", "Bumble"],
  );

  const beat = waitForWelcomeKickoffBeat({ waitMs: 5 });
  events.push(
    relayEvent({
      id: "honey-intro",
      pubkey: honey.pubkey,
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
    ["Bumble"],
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
  pubkey: fizz.pubkey,
  tags: [["client", "buzz-welcome-kickoff.opener.v1"]],
});

// The bug this branch fixes: teammate intros are thread replies, which the
// channel window excludes from the main timeline. So the kickoff saw the
// opener and never the intros, and the closer stalled until the user happened
// to click into the thread. Merging the opener's subtree in is the fix.
test("intro replies reach the closer classification without the user opening the thread", () => {
  const agentSet = { lead: fizz, teammates: [honey, bumble] };
  const channelEvents = [kickoffOpener];
  const openerReplies = [
    introReply("honey-intro", honey.pubkey, kickoffOpener.id),
    introReply("bumble-intro", bumble.pubkey, kickoffOpener.id),
  ];

  // Pin the pre-fix behaviour: on the channel events alone, both teammates
  // look silent forever. This is what stalled the closer.
  assert.deepEqual(
    classifyWelcomeKickoffResolution(
      channelEvents,
      kickoffOpener,
      agentSet,
    ).unresolved.map((agent) => agent.name),
    ["Honey", "Bumble"],
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
  const honeyIntro = introReply("honey-intro", honey.pubkey, kickoffOpener.id);
  // An open thread feeds the same replies in through both sources.
  const merged = mergeKickoffEvents(
    [kickoffOpener, honeyIntro],
    [honeyIntro, introReply("bumble-intro", bumble.pubkey, kickoffOpener.id)],
  );

  assert.deepEqual(
    merged.map((event) => event.id),
    ["opener", "honey-intro", "bumble-intro"],
  );
});

test("merging with no subtree replies leaves the channel events untouched", () => {
  const channelEvents = [kickoffOpener];
  assert.equal(mergeKickoffEvents(channelEvents, []), channelEvents);
});
