import type { TranscriptItem } from "./agentSessionTypes";

const sessionId = "debug-session-render-classes";
const turnId = "debug-turn-render-classes";
const channelId = "debug-channel-render-classes";
const baseTimestamp = Date.parse("2026-06-30T00:00:00.000Z");

function timestamp(seconds: number) {
  return new Date(baseTimestamp + seconds * 1000).toISOString();
}

export function debugPlanUpdateItem(
  id: string,
  text: string,
  seconds: number,
): Extract<TranscriptItem, { type: "plan" }> {
  return {
    id,
    type: "plan",
    renderClass: "plan",
    title: "Plan updated",
    text,
    timestamp: timestamp(seconds),
    acpSource: "plan",
    turnId,
    sessionId,
    channelId,
  };
}
