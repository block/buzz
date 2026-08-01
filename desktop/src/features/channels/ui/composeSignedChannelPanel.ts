import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
} from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import type {
  PanelManifest,
  PanelSection,
  PanelSourceEvent,
  PanelStatus,
  SignedChannelPanelState,
} from "./signedChannelPanelTypes";

const JOB_KINDS = new Set([
  KIND_JOB_REQUEST,
  KIND_JOB_ACCEPTED,
  KIND_JOB_PROGRESS,
  KIND_JOB_RESULT,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
]);
const EVENT_ID_RE = /^[0-9a-f]{64}$/;
const CHANNEL_ID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const MAX_SOURCE_EVENTS = 64;
const MAX_NOTE_LENGTH = 512;

const JOB_LABELS: Record<number, string> = {
  [KIND_JOB_REQUEST]: "Job requested",
  [KIND_JOB_ACCEPTED]: "Job accepted",
  [KIND_JOB_PROGRESS]: "Progress update",
  [KIND_JOB_RESULT]: "Job result",
  [KIND_JOB_CANCEL]: "Job cancellation",
  [KIND_JOB_ERROR]: "Job failed",
};

/**
 * Compose a bounded panel projection from already-verified job events.
 *
 * This intentionally does not interpret job content or introduce domain
 * semantics. The latest Buzz job kind supplies the presentation status; the
 * signed event ids remain the panel's provenance.
 */
export function composeSignedChannelPanelState(
  channelId: string,
  events: readonly RelayEvent[],
): SignedChannelPanelState {
  if (!CHANNEL_ID_RE.test(channelId)) {
    return { kind: "empty" };
  }

  const sourceEvents = events
    .filter(
      (event) =>
        JOB_KINDS.has(event.kind) &&
        event.pending !== true &&
        event.sig.length > 0 &&
        EVENT_ID_RE.test(event.id) &&
        getChannelIdFromTags(event.tags) === channelId &&
        Number.isSafeInteger(event.created_at) &&
        event.created_at > 0,
    )
    .sort(
      (left, right) =>
        right.created_at - left.created_at || right.id.localeCompare(left.id),
    )
    .slice(0, MAX_SOURCE_EVENTS);

  if (sourceEvents.length === 0) {
    return {
      kind: "empty",
      message: "No signed job activity has been published in this channel yet.",
    };
  }

  const latest = sourceEvents[0];
  const status = statusForJobKind(latest.kind);
  const sourceEventsForManifest = sourceEvents.map((event) =>
    toSourceEvent(event, channelId),
  );
  const manifest: PanelManifest = {
    schemaVersion: 1,
    panelId: `${channelId}:signed-work-activity`,
    channelId,
    title: "Signed work activity",
    description:
      "A read-only projection of signed job lifecycle events in this channel.",
    status,
    updatedAt: latest.created_at,
    sections: [
      buildActivitySection(
        status,
        latest,
        sourceEvents.length,
        sourceEventsForManifest,
      ),
    ],
    sourceEvents: sourceEventsForManifest,
  };

  return { kind: "ready", manifest };
}

function buildActivitySection(
  status: PanelStatus,
  latest: RelayEvent,
  eventCount: number,
  sourceEvents: PanelSourceEvent[],
): PanelSection {
  return {
    id: "recent-work",
    title: "Recent signed work",
    status,
    fields: [
      { label: "Status", value: status, presentation: "status" },
      { label: "Events", value: String(eventCount), presentation: "text" },
      {
        label: "Latest event",
        value: labelForJobKind(latest.kind),
        presentation: "text",
      },
      {
        label: "Latest update",
        value: String(latest.created_at),
        presentation: "timestamp",
      },
      {
        label: "Latest note",
        value: plainTextNote(latest.content),
        presentation: "text",
      },
    ],
    links: [
      {
        label: "Open latest source",
        target: "event",
        sourceEventId: sourceEvents[0]?.eventId,
      },
    ],
  };
}

function toSourceEvent(event: RelayEvent, channelId: string): PanelSourceEvent {
  return {
    eventId: event.id,
    kind: event.kind,
    channelId,
    label: labelForJobKind(event.kind),
  };
}

function statusForJobKind(kind: number): PanelStatus {
  switch (kind) {
    case KIND_JOB_REQUEST:
      return "pending";
    case KIND_JOB_ACCEPTED:
    case KIND_JOB_PROGRESS:
      return "active";
    case KIND_JOB_RESULT:
      return "complete";
    case KIND_JOB_CANCEL:
      return "blocked";
    case KIND_JOB_ERROR:
      return "failed";
    default:
      return "unavailable";
  }
}

function labelForJobKind(kind: number) {
  return JOB_LABELS[kind] ?? "Signed job event";
}

function plainTextNote(content: string) {
  const note = content
    .split("")
    .map((character) => {
      const code = character.charCodeAt(0);
      return code < 32 || code === 127 ? " " : character;
    })
    .join("")
    .replace(/\s+/g, " ")
    .trim();
  if (!note) return "No note provided.";
  return note.length > MAX_NOTE_LENGTH
    ? `${note.slice(0, MAX_NOTE_LENGTH - 1)}…`
    : note;
}
