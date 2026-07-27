import type { ObserverEvent } from "./agentSessionTypes";
import { asRecord, asString } from "./agentSessionUtils";

export type MentionSlaTranscriptItem = {
  id: string;
  renderClass: "status" | "error";
  title: string;
  text: string;
  channelId: string | null;
};

export function describeMentionSlaEvent(
  event: ObserverEvent,
): MentionSlaTranscriptItem | null {
  const payload = asRecord(event.payload);
  const mentionId = asString(payload.eventId) ?? String(event.seq);
  const channelId = asString(payload.channelId) ?? event.channelId ?? null;

  if (event.kind === "mention_received") {
    const dispatchState =
      asString(payload.dispatchState) ?? "accepted_for_dispatch";
    const deadlineAt = asString(payload.deadlineAt);
    const dispatchText =
      dispatchState === "waiting_for_runtime"
        ? "Waiting for the agent runtime"
        : dispatchState === "queued_behind_active_turn"
          ? "Queued behind the active turn"
          : "Accepted for dispatch";
    return {
      id: `mention-received:${mentionId}`,
      renderClass: "status",
      title: "Mention received",
      text: deadlineAt
        ? `${dispatchText}. Response deadline: ${deadlineAt}.`
        : `${dispatchText}.`,
      channelId,
    };
  }

  if (event.kind === "mention_response_sla") {
    const status = asString(payload.status) ?? "unknown";
    const reason = asString(payload.reason);
    const reasonText =
      reason === "no_relay_accepted_response"
        ? "No relay-accepted response arrived within 60 seconds."
        : reason === "relay_disconnected_before_response"
          ? "The relay disconnected before response timing could be verified."
          : reason === "pending_tracker_capacity_exceeded"
            ? "Response timing could not be retained because the pending tracker reached capacity."
            : "Response timing could not be verified.";
    return {
      id: `mention-sla:${mentionId}:${status}`,
      renderClass: status === "breached" ? "error" : "status",
      title:
        status === "breached"
          ? "Response SLA breached"
          : "Response status unknown",
      text: reasonText,
      channelId,
    };
  }

  if (event.kind !== "first_response_accepted") return null;

  const status = asString(payload.status) ?? "unknown";
  const elapsedMs =
    typeof payload.elapsedMs === "number" ? payload.elapsedMs : null;
  const elapsedText =
    elapsedMs == null ? "" : ` after ${(elapsedMs / 1_000).toFixed(1)} seconds`;
  return {
    id: `mention-response:${mentionId}`,
    renderClass: status === "breached" ? "error" : "status",
    title:
      status === "on_time"
        ? "Response accepted"
        : status === "breached"
          ? "Late response accepted"
          : "Response timing unknown",
    text: `The relay accepted the first response${elapsedText}.`,
    channelId,
  };
}
