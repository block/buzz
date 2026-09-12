import assert from "node:assert/strict";
import test from "node:test";
import {
  conversationEfforts,
  matchingEffortStatus,
  retainSessionConfigs,
} from "./conversationEffort.ts";

const native = (
  value = "medium",
  options = [
    { value: "low", name: "Low" },
    { value: "medium", name: "Medium" },
  ],
) => ({
  liveEffortSwitching: true,
  effortSessionToken: "fbf7259f-74df-4859-8b53-c0d8b77fb21e",
  configOptions: [
    {
      id: "reasoning_effort",
      category: "thought_level",
      type: "select",
      currentValue: value,
      options,
    },
  ],
});
const frame = (patch = {}) => ({
  seq: 1,
  timestamp: "2026-09-06T00:00:00Z",
  kind: "session_config_captured",
  agentIndex: 0,
  channelId: "channel",
  sessionId: "session",
  turnId: null,
  payload: native(),
  ...patch,
});

test("effort inventory is scoped to exact sessions and reads native values", () => {
  const choices = conversationEfforts(
    [
      frame(),
      frame({ sessionId: "sibling", payload: native("low") }),
      frame({ channelId: "foreign" }),
    ],
    "channel",
  );
  assert.equal(choices.length, 2);
  assert.equal(choices.find((c) => c.sessionId === "session").value, "medium");
  assert.deepEqual(
    choices[0].options.map((o) => o.value),
    ["low", "medium"],
  );
});
test("newer unsupported snapshot removes control instead of reviving stale capabilities", () => {
  assert.deepEqual(
    conversationEfforts(
      [frame({ seq: 2, payload: { configOptions: [] } }), frame()],
      "channel",
    ),
    [],
  );
});
test("replayed snapshots cannot overwrite newer applied value; grouped choices remain exact", () => {
  const choices = conversationEfforts(
    [
      frame({
        seq: 2,
        payload: native("ultra", [
          { name: "Advanced", options: [{ value: "ultra", name: "Ultra" }] },
        ]),
      }),
      frame(),
    ],
    "channel",
  );
  assert.equal(choices[0].value, "ultra");
  assert.deepEqual(choices[0].options, [{ value: "ultra", label: "Ultra" }]);
});
test("an acknowledgement must match type, request, channel, session, and effort", () => {
  const request = {
    requestId: "new",
    channelId: "channel",
    sessionId: "session",
    sessionToken: "current-session",
    effort: "high",
  };
  const result = { ...request, type: "switch_effort", status: "applied" };
  assert.equal(matchingEffortStatus(result, request), "applied");
  for (const [key, value] of Object.entries({
    requestId: "old",
    sessionToken: "previous-session",
    channelId: "foreign",
    sessionId: "sibling",
    effort: "low",
    type: "switch_model",
  })) {
    assert.equal(
      matchingEffortStatus({ ...result, [key]: value }, request),
      null,
      key,
    );
  }
  assert.equal(
    matchingEffortStatus({ ...result, status: "queued" }, request),
    "queued",
  );
});

test("old harnesses do not advertise a live control", () => {
  assert.deepEqual(
    conversationEfforts(
      [frame({ payload: { ...native(), liveEffortSwitching: undefined } })],
      "channel",
    ),
    [],
  );
});
test("retained config survives transcript eviction and strips unrelated inventories", () => {
  const retained = retainSessionConfigs(
    [],
    [frame({ payload: { ...native(), models: ["large catalog"] } })],
  );
  assert.equal(
    retainSessionConfigs(retained, [frame({ kind: "turn_started" })]),
    retained,
  );
  assert.equal(conversationEfforts(retained, "channel")[0].value, "medium");
  assert.equal(retained[0].payload.models, undefined);
});
test("rotation supersedes an old target and late replay cannot resurrect it", () => {
  const previous = frame({
    payload: { ...native(), conversationId: "thread-root" },
  });
  const next = frame({
    sessionId: "replacement",
    seq: 2,
    payload: { ...native("low"), conversationId: "thread-root" },
  });
  const retained = retainSessionConfigs(retainSessionConfigs([], [previous]), [
    next,
  ]);
  const choices = conversationEfforts([...retained, previous], "channel");
  assert.deepEqual(
    choices.map((c) => c.sessionId),
    ["replacement"],
  );
  assert.equal(retainSessionConfigs(retained, [previous]), retained);
});
test("retained sessions are bounded", () => {
  const retained = retainSessionConfigs(
    [],
    Array.from({ length: 200 }, (_, i) =>
      frame({ sessionId: `session-${i}`, seq: i }),
    ),
  );
  assert.equal(retained.length, 128);
  assert.equal(retained[0].seq, 72);
});

test("profile-wide activity keeps exact channel targets even when adapter IDs coincide", () => {
  const choices = conversationEfforts(
    [
      frame(),
      frame({
        channelId: "other",
        payload: { ...native("low"), conversationLabel: "Other channel" },
      }),
    ],
    null,
  );
  assert.equal(choices.length, 2);
  assert.deepEqual(choices.map((c) => c.channelId).sort(), [
    "channel",
    "other",
  ]);
  assert.equal(
    choices.find((c) => c.channelId === "other").label,
    "Other channel",
  );
});
