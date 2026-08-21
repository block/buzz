import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const dir = mkdtempSync(path.join(tmpdir(), "triage-prioritize-"));
const file = path.join(dir, "data.json");
writeFileSync(file, `${JSON.stringify({})}\n`);
process.env.TRIAGE_DATA_FILE = file;
process.env.TRIAGE_LLM = "0";

const { recordTranscript, resetStore } = await import("./store.mjs");
const { buildLessons } = await import("./lessons.mjs");
const {
  buildPrioritizePrompt,
  heuristicScore,
  parsePriorityScores,
  prioritizeFibres,
} = await import("./prioritize.mjs");

test.after(() => {
  resetStore();
  rmSync(dir, { recursive: true, force: true });
});

function transcript(overrides = {}) {
  return {
    eventId: "e",
    channelId: "c1",
    threadRootId: "t",
    authorPubkey: "vlad",
    authorLabel: "Vlad",
    content: "please take a look",
    createdAt: 100,
    isMention: false,
    isSelf: false,
    ...overrides,
  };
}

function fibre(overrides = {}) {
  return {
    id: "f1",
    kind: "ask",
    status: "open",
    score: 50,
    engagement: 0,
    title: "Review the migration",
    summary: "Vlad asked for a review",
    channelId: "c1",
    channelName: "engineering",
    isDm: false,
    people: [{ pubkey: "vlad", label: "Vlad" }],
    artifacts: [
      {
        eventId: "p1",
        channelId: "c1",
        threadRootId: "t",
        authorLabel: "Vlad",
        content: "please take a look",
        createdAt: 100,
      },
    ],
    ...overrides,
  };
}

test("a second unanswered ping raises the score", async () => {
  const pubkey = "escalation";
  recordTranscript(pubkey, [
    transcript({ eventId: "p1", createdAt: 100, isMention: true }),
  ]);

  const target = fibre();
  const before = await prioritizeFibres({ pubkey, fibres: [target] });

  recordTranscript(pubkey, [
    transcript({ eventId: "p2", createdAt: 200, isMention: true }),
  ]);
  const after = await prioritizeFibres({ pubkey, fibres: [target] });

  assert.ok(
    after.get("f1").score > before.get("f1").score,
    `expected ${after.get("f1").score} > ${before.get("f1").score}`,
  );
  assert.match(after.get("f1").signals[0].label, /Pinged 2 times/);
});

test("replying drops the fibre back down", async () => {
  const pubkey = "answered";
  recordTranscript(pubkey, [
    transcript({ eventId: "p1", createdAt: 100, isMention: true }),
    transcript({ eventId: "p2", createdAt: 200, isMention: true }),
  ]);
  const target = fibre();
  const pending = await prioritizeFibres({ pubkey, fibres: [target] });

  recordTranscript(pubkey, [
    transcript({ eventId: "mine", createdAt: 300, isSelf: true }),
  ]);
  const answered = await prioritizeFibres({ pubkey, fibres: [target] });

  assert.ok(answered.get("f1").score < pending.get("f1").score);
});

test("a busy thread lands in the hot lane without being important", async () => {
  const pubkey = "hot";
  const rows = Array.from({ length: 10 }, (_, index) =>
    transcript({
      eventId: `m${index}`,
      createdAt: 1_000 + index * 30,
      authorPubkey: `person-${index % 4}`,
    }),
  );
  recordTranscript(pubkey, rows);

  const result = await prioritizeFibres({
    pubkey,
    fibres: [
      fibre({
        kind: "fyi",
        artifacts: [
          {
            eventId: "m0",
            channelId: "c1",
            threadRootId: "t",
            content: "thinking out loud about the router",
            createdAt: 1_000,
          },
        ],
      }),
    ],
  });

  const scored = result.get("f1");
  assert.ok(scored.engagement >= 60, `engagement was ${scored.engagement}`);
  assert.ok(scored.score <= 70, `score was ${scored.score}`);
  assert.equal(scored.lane, "hot");
});

test("the heuristic keeps a plain mention out of the important lane", () => {
  const quiet = {
    mentionCount: 1,
    isDm: false,
    velocity: { messages: 1, participants: 1 },
    pastInvolvement: { any: false },
  };

  assert.equal(heuristicScore(fibre(), quiet, buildLessons([])), 62);
  assert.equal(
    heuristicScore(fibre({ kind: "blocker" }), quiet, buildLessons([])),
    75,
  );
  assert.equal(
    heuristicScore(fibre({ kind: "fyi" }), { ...quiet, mentionCount: 0 }, buildLessons([])),
    30,
  );
});

test("heuristicScore folds in what the user kept and dismissed", () => {
  const lessons = buildLessons([
    { userAction: "dismissed", authorPubkey: "vlad", preview: "x" },
    { userAction: "dismissed", authorPubkey: "vlad", preview: "y" },
  ]);
  const signals = {
    mentionCount: 1,
    isDm: false,
    velocity: { messages: 1, participants: 1 },
    pastInvolvement: { any: false },
  };

  assert.equal(heuristicScore(fibre(), signals, lessons), 62 - 16);
});

test("parsePriorityScores drops rows without a usable score", () => {
  const scores = parsePriorityScores({
    scores: [
      { fibreId: "a", score: 150, why: "Way too high" },
      { fibreId: "b", score: "nope" },
      { score: 40 },
    ],
  });

  assert.equal(scores.size, 1);
  assert.equal(scores.get("a").score, 100);
});

test("the stage 2 prompt states the importance rules and the computed facts", () => {
  const prompt = buildPrioritizePrompt({
    fibres: [fibre()],
    signalsById: new Map([
      [
        "f1",
        {
          mentionCount: 1,
          unansweredPings: 2,
          hasReplyFromViewer: false,
          velocity: { messages: 4, participants: 2 },
          pastInvolvement: { any: true },
          incidentChannel: false,
          incidentOngoing: false,
          followUpOnly: false,
          engagement: 36,
        },
      ],
    ]),
    lessons: { examples: [] },
  });

  assert.match(prompt, /"unansweredPings":2/);
  assert.match(prompt, /"youWorkedOnThisBefore":true/);
  assert.match(prompt, /Blast radius decides the top of the scale/);
  assert.match(prompt, /lunch orders/);
  assert.match(prompt, /does not by itself make it important/);
  assert.match(prompt, /Score importance only/);
});
