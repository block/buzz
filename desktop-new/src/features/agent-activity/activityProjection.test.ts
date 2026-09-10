import { describe, expect, it } from "vitest";

import { reduceActivity } from "./activityProjection";

const event = (kind: string, status: string) => ({
  seq: 1,
  timestamp: "2026-09-08T03:00:00.000Z",
  kind,
  channelId: "design",
  sessionId: "session",
  turnId: "turn",
  agentPubkey: "agent",
  payload: { agentName: "Vogue", title: kind, status },
});

describe("reduceActivity", () => {
  it("keeps a turn working after an individual tool completes", () => {
    const started = reduceActivity(new Map(), event("turn_started", "running"));
    const updated = reduceActivity(
      started,
      event("tool_call_completed", "completed"),
    );

    expect([...updated.values()][0]?.status).toBe("running");
  });

  it("settles only when the turn itself reaches a terminal event", () => {
    const started = reduceActivity(new Map(), event("turn_started", "running"));
    const completed = reduceActivity(
      started,
      event("turn_completed", "completed"),
    );

    expect([...completed.values()][0]?.status).toBe("completed");
  });
});
