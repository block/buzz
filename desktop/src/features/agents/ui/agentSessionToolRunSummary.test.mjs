import assert from "node:assert/strict";
import test from "node:test";

import {
  dominantToolRunKind,
  isToolRunEligible,
  summarizeToolRunHeadline,
  summarizeToolRunStatus,
  TOOL_RUN_MINIMUM_STEPS,
  toolRunCompletedAtMs,
  toolRunElapsedMs,
  toolRunGroupKey,
  toolRunKind,
  toolRunStartedAtMs,
} from "./agentSessionToolRunSummary.ts";
import { classifyTool } from "./agentSessionToolClassifier.ts";

const START = "2026-06-18T00:00:00.000Z";
const START_MS = Date.parse(START);

function tool(id, renderClass, overrides = {}) {
  return {
    id,
    type: "tool",
    renderClass,
    descriptor: {
      renderClass,
      label: "Ran tool",
      preview: id,
      source: "harness",
      groupKey: renderClass,
    },
    title: id,
    toolName: id,
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: START,
    startedAt: START,
    completedAt: "2026-06-18T00:00:01.000Z",
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
    ...overrides,
  };
}

function headlineOf(items) {
  return summarizeToolRunHeadline(items, summarizeToolRunStatus(items));
}

/**
 * A tool item whose descriptor comes from the real classifier, so `action` and
 * `tone` are whatever the production taxonomy actually derives. Used wherever a
 * test is about the run consuming classifier semantics rather than about the
 * run's own arithmetic.
 */
function classified(id, { args = {}, toolName, title, isError = false }) {
  const input = {
    title: title ?? toolName,
    toolName,
    buzzToolName: null,
    args,
    result: "",
    isError,
  };
  const descriptor = classifyTool(input);
  return tool(id, descriptor.renderClass, {
    args,
    descriptor,
    isError,
    status: isError ? "failed" : "completed",
    title: input.title,
    toolName,
  });
}

/** A `buzz` CLI step, whose tone the classifier derives from group + verb. */
function buzzCli(id, command, overrides = {}) {
  return classified(id, {
    args: { command },
    title: "shell",
    toolName: "shell",
    ...overrides,
  });
}

// ── Eligibility ──────────────────────────────────────────────────────────────

test("isToolRunEligible admits ordinary tool work", () => {
  for (const renderClass of [
    "file-read",
    "file-edit",
    "relay-op",
    "skill-read",
    "message",
    "generic",
    "image",
    "shell",
    "plan",
  ]) {
    assert.equal(isToolRunEligible(tool("t", renderClass)), true, renderClass);
  }
});

// The safety net and every intervention point must stay visible as its own row.
test("isToolRunEligible refuses raw-rail, suppressed, status, permission, thought", () => {
  for (const renderClass of [
    "raw-rail",
    "suppressed",
    "status",
    "permission",
    "thought",
  ]) {
    assert.equal(isToolRunEligible(tool("t", renderClass)), false, renderClass);
  }
});

test("isToolRunEligible admits a failed step so it stays in its own run", () => {
  assert.equal(
    isToolRunEligible(tool("t", "error", { isError: true, status: "failed" })),
    true,
  );
  assert.equal(
    isToolRunEligible(tool("t", "shell", { isError: true, status: "failed" })),
    true,
  );
});

// A failure must not launder an excluded class into an eligible one. The
// classifier flattens every failed step to render class `error` while keeping
// its original groupKey, so deciding eligibility on the reported class alone
// pulled failed safety/status rows into chains. Built through the real
// classifier, because the bug lived in exactly that flattening.
test("a failed suppressed or status step is still refused", () => {
  const failedStopHook = classified("s", { isError: true, toolName: "stop" });
  assert.equal(failedStopHook.descriptor.renderClass, "error");
  assert.equal(failedStopHook.descriptor.groupKey, "suppressed:stop-hook");
  assert.equal(isToolRunEligible(failedStopHook), false);

  const failedCompact = classified("c", {
    isError: true,
    toolName: "postcompact",
  });
  assert.equal(failedCompact.descriptor.renderClass, "error");
  assert.equal(failedCompact.descriptor.groupKey, "status:post-compact");
  assert.equal(isToolRunEligible(failedCompact), false);
});

test("isToolRunEligible refuses non-tool items", () => {
  assert.equal(
    isToolRunEligible({
      id: "m",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Assistant",
      text: "hi",
      timestamp: START,
    }),
    false,
  );
});

test("isToolRunEligible falls back to the descriptor render class", () => {
  const item = tool("t", undefined);
  item.renderClass = undefined;
  item.descriptor.renderClass = "shell";
  assert.equal(isToolRunEligible(item), true);

  const suppressed = tool("t2", undefined);
  suppressed.renderClass = undefined;
  suppressed.descriptor.renderClass = "suppressed";
  assert.equal(isToolRunEligible(suppressed), false);
});

test("toolRunGroupKey prefers the descriptor groupKey, falling back to the class", () => {
  assert.equal(
    toolRunGroupKey(
      tool("t", "file-read", {
        descriptor: {
          renderClass: "file-read",
          label: "Read file",
          preview: null,
          groupKey: "read_file",
        },
      }),
    ),
    "read_file",
  );
  assert.equal(
    toolRunGroupKey(
      tool("t", "file-read", {
        descriptor: {
          renderClass: "file-read",
          label: "Read file",
          preview: null,
        },
      }),
    ),
    "file-read",
  );
});

test("TOOL_RUN_MINIMUM_STEPS keeps a lone step out of a card", () => {
  assert.equal(TOOL_RUN_MINIMUM_STEPS, 2);
});

// ── Aggregate status ─────────────────────────────────────────────────────────

test("summarizeToolRunStatus reports done for an all-settled clean run", () => {
  const aggregate = summarizeToolRunStatus([
    tool("a", "shell"),
    tool("b", "shell"),
  ]);
  assert.deepEqual(aggregate, {
    phase: "done",
    hasError: false,
    count: 2,
    activeStep: null,
    errorCount: 0,
  });
});

test("summarizeToolRunStatus reports the FIRST unsettled step as active", () => {
  const aggregate = summarizeToolRunStatus([
    tool("a", "shell"),
    tool("b", "shell", { status: "executing", completedAt: null }),
    tool("c", "shell", { status: "pending", completedAt: null }),
  ]);
  assert.equal(aggregate.phase, "running");
  assert.equal(aggregate.activeStep, 2);
});

test("summarizeToolRunStatus counts failures from isError or failed status", () => {
  const aggregate = summarizeToolRunStatus([
    tool("a", "shell"),
    tool("b", "error", { isError: true }),
    tool("c", "shell", { status: "failed" }),
  ]);
  assert.equal(aggregate.phase, "error");
  assert.equal(aggregate.hasError, true);
  assert.equal(aggregate.errorCount, 2);
});

// A live run keeps reporting that work is ongoing; the failure is not masked —
// hasError stays true so the card stays open and highlights the failing step.
test("summarizeToolRunStatus keeps running phase while a failed step is followed by live work", () => {
  const aggregate = summarizeToolRunStatus([
    tool("a", "error", { isError: true }),
    tool("b", "shell", { status: "executing", completedAt: null }),
  ]);
  assert.equal(aggregate.phase, "running");
  assert.equal(aggregate.hasError, true);
});

// ── Kind (render class + tone) ────────────────────────────────────────────────

test("toolRunKind carries the classifier's tone, not just the render class", () => {
  assert.deepEqual(
    toolRunKind(buzzCli("r", "buzz messages get --channel abc")),
    {
      renderClass: "relay-op",
      tone: "read",
    },
  );
  assert.deepEqual(toolRunKind(buzzCli("w", "buzz canvas set --channel abc")), {
    renderClass: "relay-op",
    tone: "write",
  });
  assert.deepEqual(toolRunKind(buzzCli("a", "buzz channels create --name x")), {
    renderClass: "relay-op",
    tone: "admin",
  });
});

// The classifier flattens a failed step to renderClass "error" with a
// "… failed" label, which would otherwise erase what the step was doing and
// make any run containing one headline as anonymous tool work.
test("toolRunKind recovers a failed step's original class and tone", () => {
  const failedRead = buzzCli("f", "buzz messages get --channel abc", {
    isError: true,
  });
  assert.equal(failedRead.descriptor.renderClass, "error");
  assert.deepEqual(toolRunKind(failedRead), {
    renderClass: "relay-op",
    tone: "read",
  });

  const failedFileRead = classified("f2", {
    args: { path: "a.ts" },
    isError: true,
    toolName: "read_file",
  });
  assert.equal(toolRunKind(failedFileRead).renderClass, "file-read");
});

// ── Headlines ────────────────────────────────────────────────────────────────

test("summarizeToolRunHeadline keeps the specific countable sentence for a homogeneous run", () => {
  const read = (id) =>
    tool(id, "file-read", {
      descriptor: {
        renderClass: "file-read",
        label: "Read file",
        preview: null,
        action: { verb: "Read", object: "a.ts" },
        groupKey: "read_file",
      },
    });

  assert.deepEqual(headlineOf([read("a"), read("b"), read("c")]), {
    verb: "Read",
    object: "3 files",
    detail: null,
  });
});

test("summarizeToolRunHeadline names each homogeneous run class", () => {
  const homogeneous = (renderClass, groupKey, verb) =>
    [1, 2].map((n) =>
      tool(`${renderClass}-${n}`, renderClass, {
        descriptor: {
          renderClass,
          label: "Label",
          preview: null,
          action: { verb, object: null },
          groupKey,
        },
      }),
    );

  assert.deepEqual(headlineOf(homogeneous("file-edit", "edit", "Edited")), {
    verb: "Edited",
    object: "2 files",
    detail: null,
  });
  assert.deepEqual(headlineOf(homogeneous("skill-read", "skill", "Read")), {
    verb: "Read",
    object: "2 skills",
    detail: null,
  });
  assert.deepEqual(headlineOf(homogeneous("shell", "cmd", "Ran")), {
    verb: "Ran",
    object: "2 commands",
    detail: null,
  });
  assert.deepEqual(headlineOf(homogeneous("message", "msg", "Sent")), {
    verb: "Sent",
    object: "2 messages",
    detail: null,
  });
  assert.deepEqual(headlineOf(homogeneous("image", "img", "Viewed")), {
    verb: "Viewed",
    object: "2 images",
    detail: null,
  });
  assert.deepEqual(headlineOf(homogeneous("plan", "todo", "Updated")), {
    verb: "Updated",
    object: "2 todos",
    detail: null,
  });
});

// The verb comes from the descriptors the classifier already produced, so two
// runs of the same render class read differently when their tone differs.
test("summarizeToolRunHeadline takes a homogeneous run's verb from the classifier", () => {
  assert.deepEqual(
    headlineOf([
      buzzCli("r1", "buzz messages get --channel abc"),
      buzzCli("r2", "buzz messages get --channel def"),
    ]),
    { verb: "Read", object: "2 Buzz relay ops", detail: null },
  );
  assert.deepEqual(
    headlineOf([
      buzzCli("w1", "buzz canvas set --channel abc"),
      buzzCli("w2", "buzz canvas set --channel def"),
    ]),
    { verb: "Updated", object: "2 Buzz relay ops", detail: null },
  );
  assert.deepEqual(
    headlineOf([
      buzzCli("a1", "buzz channels create --name x"),
      buzzCli("a2", "buzz channels create --name y"),
    ]),
    { verb: "Created", object: "2 Buzz relay ops", detail: null },
  );
});

test("summarizeToolRunHeadline falls back to the descriptor label for an unnamed homogeneous run", () => {
  const generic = (id) =>
    tool(id, "generic", {
      descriptor: {
        renderClass: "generic",
        label: "Queried registry",
        preview: null,
        action: { verb: "Ran", object: "registry" },
        groupKey: "registry",
      },
    });

  assert.deepEqual(headlineOf([generic("a"), generic("b")]), {
    verb: "Queried registry",
    object: "×2",
    detail: null,
  });
});

// A heterogeneous run must not pretend to be one thing: it names the dominant
// kind of work and carries the honest step count.
test("summarizeToolRunHeadline names the dominant kind and step count for a mixed run", () => {
  const headline = headlineOf([
    classified("read-1", { args: { path: "a.ts" }, toolName: "read_file" }),
    classified("read-2", { args: { path: "b.ts" }, toolName: "read_file" }),
    buzzCli("shell-1", "ls -la"),
  ]);

  assert.deepEqual(headline, {
    verb: "Read",
    object: "files",
    detail: "3 steps",
  });
});

// The regression this guards: keying the headline on render class alone
// flattened every heterogeneous relay-op run to one generic phrase, so a sweep
// of reads and a sweep of admin changes were indistinguishable.
test("a mixed relay-op run reflects read vs write vs admin tone", () => {
  const reads = headlineOf([
    buzzCli("r1", "buzz messages get --channel abc"),
    buzzCli("r2", "buzz channels list"),
    buzzCli("r3", "buzz feed get"),
  ]);
  assert.deepEqual(reads, {
    verb: "Read",
    object: "Buzz relay ops",
    detail: "3 steps",
  });

  const writes = headlineOf([
    buzzCli("w1", "buzz canvas set --channel abc"),
    buzzCli("w2", "buzz reactions add --event e1"),
    buzzCli("w3", "buzz messages get --channel abc"),
  ]);
  assert.deepEqual(writes, {
    // Two write-toned ops outnumber the one read; their verbs disagree
    // ("Updated"/"Added"), so the run falls back to the write tone's verb.
    verb: "Updated",
    object: "Buzz relay ops",
    detail: "3 steps",
  });

  const admin = headlineOf([
    buzzCli("a1", "buzz channels create --name x"),
    buzzCli("a2", "buzz channels delete --id y"),
    buzzCli("a3", "buzz messages get --channel abc"),
  ]);
  assert.deepEqual(admin, {
    verb: "Changed",
    object: "Buzz relay ops",
    detail: "3 steps",
  });
});

// Equal counts of read- and admin-toned relay work: the more consequential tone
// headlines the run.
test("an evenly split relay run headlines with the more salient tone", () => {
  assert.deepEqual(
    headlineOf([
      buzzCli("r1", "buzz messages get --channel abc"),
      buzzCli("a1", "buzz channels create --name x"),
    ]),
    { verb: "Created", object: "Buzz relay ops", detail: "2 steps" },
  );
});

// A failed step keeps its class, so one failed read inside a run of reads still
// reads as a run of reads rather than as anonymous tool work.
test("a failed step does not drag a run's headline to generic tool work", () => {
  assert.deepEqual(
    headlineOf([
      classified("read-1", { args: { path: "a.ts" }, toolName: "read_file" }),
      classified("read-2", {
        args: { path: "b.ts" },
        isError: true,
        toolName: "read_file",
      }),
      classified("read-3", { args: { path: "c.ts" }, toolName: "read_file" }),
    ]),
    { verb: "Read", object: "3 files", detail: null },
  );
});

test("summarizeToolRunHeadline reads as active with the step position while live", () => {
  const read = (id, overrides = {}) =>
    tool(id, "file-read", {
      descriptor: {
        renderClass: "file-read",
        label: "Read file",
        preview: null,
        action: { verb: "Read", object: "a.ts" },
        groupKey: "read_file",
      },
      ...overrides,
    });

  assert.deepEqual(
    headlineOf([
      read("read-1"),
      read("read-2"),
      read("read-3", { status: "executing", completedAt: null }),
    ]),
    { verb: "Reviewing", object: "files", detail: "step 3" },
  );
});

// The live header is the settled header in another tense, off the same verb —
// not a second vocabulary.
test("a live run re-tenses the classifier's verb", () => {
  const live = (item) => ({ ...item, status: "executing", completedAt: null });

  assert.equal(
    headlineOf([
      buzzCli("w1", "buzz canvas set --channel abc"),
      live(buzzCli("w2", "buzz canvas set --channel def")),
    ]).verb,
    "Updating",
  );
  assert.equal(
    headlineOf([buzzCli("c1", "ls -la"), live(buzzCli("c2", "pwd"))]).verb,
    "Running",
  );
  assert.equal(
    headlineOf([
      buzzCli("a1", "buzz channels create --name x"),
      live(buzzCli("a2", "buzz channels create --name y")),
    ]).verb,
    "Creating",
  );
});

// The bug this guards: mixed admin relay ops disagree on a verb, so the run
// falls back to the admin tone's verb ("Changed") — which had no progressive
// form, leaving a LIVE card reading in the past tense. Built through the real
// classifier so the tone and the disagreement are production behaviour.
test("a live run of mixed admin relay ops still reads present-tense", () => {
  const live = (item) => ({ ...item, status: "executing", completedAt: null });

  const created = buzzCli("a1", "buzz channels create --name x");
  const deleted = live(buzzCli("a2", "buzz channels delete --channel y"));

  // Precondition: genuinely admin-toned, and the two steps really do disagree
  // on a verb — otherwise the single agreed verb would be used and the tone
  // fallback (the thing under test) would never be reached.
  assert.equal(toolRunKind(created).tone, "admin");
  assert.equal(toolRunKind(deleted).tone, "admin");
  assert.notEqual(
    created.descriptor.action.verb,
    deleted.descriptor.action.verb,
  );

  const headline = headlineOf([created, deleted]);
  assert.equal(headline.verb, "Changing");
  assert.equal(headline.detail, "step 2");
  // The regression, stated directly: no live header reads in the past tense.
  assert.notEqual(headline.verb, "Changed");
});

// Each tone's fallback verb, exercised through the public headline rather than
// by reaching for the module-private tables.
//
// A run only falls back to its tone's verb when the steps that DEFINE the
// headline disagree on one — and those are just the steps sharing the dominant
// render class and tone. So each pair below shares a class and a tone and
// differs only in verb; a pair that differed in class instead would narrow the
// dominant kind to a single step, whose verb would trivially agree, and the
// fallback under test would never be reached.
test("every tone's fallback verb re-tenses for a live run", () => {
  const live = (item) => ({ ...item, status: "executing", completedAt: null });
  const verbOf = (a, b) => {
    // Precondition for every case: same kind, disagreeing verbs. Asserted so a
    // fixture that stops reaching the fallback fails loudly instead of passing
    // for the wrong reason.
    assert.deepEqual(toolRunKind(a), toolRunKind(b));
    assert.notEqual(a.descriptor.action.verb, b.descriptor.action.verb);
    return headlineOf([a, live(b)]).verb;
  };

  // read: get ("Read") vs search ("Searched") → TONE_VERB.read, re-tensed.
  assert.equal(
    verbOf(
      buzzCli("r1", "buzz messages get --channel abc"),
      buzzCli("r2", "buzz messages search --query hi"),
    ),
    "Reviewing",
  );
  // write: canvas set ("Updated") vs reactions add ("Added").
  assert.equal(
    verbOf(
      buzzCli("w1", "buzz canvas set --channel abc --content x"),
      buzzCli("w2", "buzz reactions add --event e --emoji tada"),
    ),
    "Updating",
  );
  // admin: create ("Created") vs delete ("Deleted") → "Changed" → "Changing".
  // This is the case that shipped a past-tense verb on a live card.
  assert.equal(
    verbOf(
      buzzCli("a1", "buzz channels create --name x"),
      buzzCli("a2", "buzz channels delete --channel y"),
    ),
    "Changing",
  );
  // neutral has no tone verb at all, so it falls through to the render class's
  // floor. The classifier gives each neutral-toned class one verb, so a
  // disagreeing pair has to be built by hand to reach the floor.
  const imageStep = (id, verb, groupKey) =>
    tool(id, "image", {
      descriptor: {
        renderClass: "image",
        label: "Viewed image",
        preview: null,
        action: { verb, object: null },
        groupKey,
      },
    });
  const viewed = imageStep("i1", "Viewed", "view_image");
  assert.equal(toolRunKind(viewed).tone, "neutral");
  assert.equal(
    verbOf(viewed, imageStep("i2", "Captured", "screenshot")),
    "Viewing",
  );
});

test("summarizeToolRunHeadline tolerates an empty run", () => {
  assert.deepEqual(headlineOf([]), {
    verb: "Working…",
    object: null,
    detail: null,
  });
});

// Writes and outbound speech outrank reads when equally common, so a run that
// edited as much as it read headlines as editing.
test("dominantToolRunKind breaks ties toward the more salient render class", () => {
  assert.equal(
    dominantToolRunKind([
      tool("read-1", "file-read"),
      tool("edit-1", "file-edit"),
    ]).renderClass,
    "file-edit",
  );
  assert.equal(
    dominantToolRunKind([
      tool("read-1", "file-read"),
      tool("read-2", "file-read"),
      tool("edit-1", "file-edit"),
    ]).renderClass,
    "file-read",
  );
});

test("dominantToolRunKind keeps a failed step's recovered class in the count", () => {
  assert.deepEqual(
    dominantToolRunKind([
      buzzCli("f", "buzz messages get --channel abc", { isError: true }),
    ]),
    { renderClass: "relay-op", tone: "read" },
  );
});

// ── Timing ───────────────────────────────────────────────────────────────────

test("toolRunStartedAtMs takes the earliest start across the run", () => {
  const items = [
    tool("a", "shell", { startedAt: "2026-06-18T00:00:05.000Z" }),
    tool("b", "shell", { startedAt: START }),
  ];
  assert.equal(toolRunStartedAtMs(items), START_MS);
});

test("toolRunStartedAtMs falls back to the timestamp when startedAt is absent", () => {
  const item = tool("a", "shell", { startedAt: "" });
  assert.equal(toolRunStartedAtMs([item]), START_MS);
});

test("toolRunCompletedAtMs returns null until every step has settled", () => {
  const settled = [
    tool("a", "shell", { completedAt: "2026-06-18T00:00:01.000Z" }),
    tool("b", "shell", { completedAt: "2026-06-18T00:00:04.000Z" }),
  ];
  assert.equal(
    toolRunCompletedAtMs(settled),
    Date.parse("2026-06-18T00:00:04.000Z"),
  );

  const partial = [...settled, tool("c", "shell", { completedAt: null })];
  assert.equal(toolRunCompletedAtMs(partial), null);
});

test("toolRunElapsedMs spans first start to last completion once settled", () => {
  const items = [
    tool("a", "shell", { completedAt: "2026-06-18T00:00:01.000Z" }),
    tool("b", "shell", { completedAt: "2026-06-18T00:00:04.500Z" }),
  ];
  assert.equal(toolRunElapsedMs(items, START_MS + 999_999), 4500);
});

test("toolRunElapsedMs runs to now while a step is unsettled", () => {
  const items = [
    tool("a", "shell", { completedAt: "2026-06-18T00:00:01.000Z" }),
    tool("b", "shell", { status: "executing", completedAt: null }),
  ];
  assert.equal(toolRunElapsedMs(items, START_MS + 2500), 2500);
});

test("toolRunElapsedMs returns null for unmeasurable or negative spans", () => {
  const unparseable = tool("a", "shell", {
    startedAt: "nope",
    timestamp: "nope",
  });
  assert.equal(toolRunElapsedMs([unparseable], START_MS), null);
  assert.equal(toolRunStartedAtMs([unparseable]), null);

  const settled = [
    tool("a", "shell", { completedAt: "2026-06-17T23:59:59.000Z" }),
  ];
  assert.equal(toolRunElapsedMs(settled, START_MS), null);
});
