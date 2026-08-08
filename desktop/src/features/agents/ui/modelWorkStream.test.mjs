import assert from "node:assert/strict";
import test from "node:test";

import { buildModelWorkStream } from "./modelWorkStream.ts";

const baseIdentity = {
  channelId: "channel-1",
  sessionId: "session-1",
  turnId: "turn-1",
};

function tool({
  id,
  label,
  renderClass = "relay-op",
  result = "",
  status = "completed",
  tone = "read",
  toolName = label,
}) {
  return {
    ...baseIdentity,
    id,
    type: "tool",
    renderClass,
    descriptor: {
      action: { verb: label, object: "channel history" },
      label,
      preview: "channel history",
      renderClass,
      tone,
    },
    title: label,
    toolName,
    buzzToolName: toolName,
    status,
    args: {},
    result,
    isError: false,
    timestamp: "2026-08-05T00:00:00.000Z",
    startedAt: "2026-08-05T00:00:00.000Z",
    completedAt: status === "completed" ? "2026-08-05T00:00:01.000Z" : null,
  };
}

test("maps context, discovery, decision, action, and delivery in order", () => {
  const stream = buildModelWorkStream(
    [
      {
        ...baseIdentity,
        id: "context",
        type: "metadata",
        renderClass: "raw-rail",
        title: "Prompt context",
        sections: [
          { title: "Workspace", body: "private instructions" },
          { title: "Channel history", body: "private channel context" },
        ],
        timestamp: "2026-08-05T00:00:00.000Z",
      },
      tool({
        id: "read",
        label: "Checked",
        result: JSON.stringify({ messages: [{ id: "1" }, { id: "2" }] }),
      }),
      {
        ...baseIdentity,
        id: "thought",
        type: "thought",
        renderClass: "thought",
        title: "Analysis",
        text: "private hidden reasoning must not surface",
        timestamp: "2026-08-05T00:00:01.000Z",
      },
      tool({ id: "edit", label: "Updated", tone: "write" }),
      tool({ id: "send", label: "Sent", renderClass: "message" }),
    ],
    { isWorking: false },
  );

  assert.deepEqual(
    stream.steps.map((step) => step.phase),
    ["context", "explore", "decide", "act", "deliver"],
  );
  assert.equal(stream.steps[1]?.finding, "2 messages returned");
  assert.equal(stream.steps[1]?.signalLabel, "Found");
  assert.equal(stream.steps[1]?.trace.name, "Checked");
  assert.equal(stream.steps[1]?.trace.output, "2 messages returned");
  assert.equal(stream.steps[2]?.label, "Analyzing available signals");
  assert.equal(
    stream.steps.some((step) =>
      `${step.label} ${step.detail}`.includes("private hidden reasoning"),
    ),
    false,
  );
  assert.equal(stream.steps[4]?.label, "Response delivered");
  assert.match(stream.focus, /Response delivered/);
});

test("marks the latest running tool and phase as active", () => {
  const stream = buildModelWorkStream(
    [
      tool({ id: "read", label: "Checked" }),
      tool({
        id: "search",
        label: "Searching",
        status: "executing",
      }),
    ],
    { isWorking: true },
  );

  assert.equal(stream.activePhase, "explore");
  assert.equal(stream.phaseStates.explore, "active");
  assert.equal(stream.steps.at(-1)?.status, "active");
  assert.match(stream.focus, /Searching/);
  assert.equal(stream.steps.at(-1)?.trace.output, "Awaiting result");
});

test("summarizes structured findings without dumping raw tool output", () => {
  const stream = buildModelWorkStream(
    [
      tool({
        id: "read",
        label: "Read",
        result: JSON.stringify({
          content: "The launch date is Friday after the final review.",
        }),
      }),
    ],
    { isWorking: false },
  );

  assert.equal(
    stream.steps[0]?.finding,
    "The launch date is Friday after the final review.",
  );
  assert.equal(stream.totals.findings, 1);
});

test("treats model sampling as a decision and surfaces its selected action", () => {
  const stream = buildModelWorkStream(
    [
      tool({
        id: "sample",
        label: "Sampled",
        result: "Selected action: check_messages.",
        toolName: "sample_model",
      }),
    ],
    { isWorking: false },
  );
  assert.equal(stream.steps[0]?.phase, "decide");
  assert.equal(stream.steps[0]?.signalLabel, "Chose");
  assert.equal(stream.steps[0]?.finding, "Selected action: check_messages.");
  assert.equal(stream.steps[0]?.trace.name, "sample_model");
});

test("keeps the actual tool arguments alongside the summarized flow", () => {
  const item = tool({ id: "messages", label: "Checked" });
  item.args = {
    channel: "agents",
    contextMessages: 24,
    endpoint: "ampersand/glm51",
    unreadOnly: true,
  };

  const stream = buildModelWorkStream([item], { isWorking: false });
  assert.equal(
    stream.steps[0]?.trace.input,
    "channel=agents · contextMessages=24 · endpoint=ampersand/glm51 · unreadOnly=true",
  );
});
