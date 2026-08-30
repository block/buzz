import type { RelayEvent } from "@/shared/api/types";
import { KIND_WORK_REPORT } from "@/shared/constants/kinds";

export const WORK_REPORT_STATUSES = [
  "completed",
  "in_review",
  "needs_decision",
  "blocked",
  "failed",
] as const;

export type WorkReportStatus = (typeof WORK_REPORT_STATUSES)[number];

export type WorkReport = {
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  status: WorkReportStatus;
  outcome: string;
  deliverables: string[];
  decisions: string[];
  verification: string[];
  risks: string[];
  nextActions: string[];
  prior: string | null;
};

const STATUS_SET = new Set<string>(WORK_REPORT_STATUSES);
const ARRAY_KEYS = [
  "deliverables",
  "decisions",
  "verification",
  "risks",
  "next_actions",
] as const;

function singleTag(event: RelayEvent, name: string): string | null {
  const values = event.tags.filter((tag) => tag[0] === name);
  return values.length === 1 ? (values[0][1] ?? null) : null;
}

function stringArray(
  body: Record<string, unknown>,
  key: (typeof ARRAY_KEYS)[number],
): string[] | null {
  const value = body[key];
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    return null;
  }
  return value;
}

export function parseWorkReport(
  event: RelayEvent,
  channelId: string,
  rootId: string,
): WorkReport | null {
  if (event.kind !== KIND_WORK_REPORT) return null;
  if (singleTag(event, "h") !== channelId) return null;
  if (singleTag(event, "t") !== "work-report") return null;
  const rootTags = event.tags.filter(
    (tag) => tag[0] === "e" && tag[3] === "root",
  );
  if (rootTags.length !== 1 || rootTags[0][1] !== rootId) return null;

  let body: Record<string, unknown>;
  try {
    const parsed: unknown = JSON.parse(event.content);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }
    body = parsed as Record<string, unknown>;
  } catch {
    return null;
  }

  const statusTag = singleTag(event, "status");
  if (
    typeof body.status !== "string" ||
    body.status !== statusTag ||
    !STATUS_SET.has(body.status)
  ) {
    return null;
  }
  if (typeof body.outcome !== "string" || body.outcome.trim().length === 0) {
    return null;
  }

  const arrays = ARRAY_KEYS.map((key) => stringArray(body, key));
  if (arrays.some((value) => value === null)) return null;
  const [deliverables, decisions, verification, risks, nextActions] =
    arrays as string[][];

  return {
    eventId: event.id,
    authorPubkey: event.pubkey,
    createdAt: event.created_at,
    status: body.status as WorkReportStatus,
    outcome: body.outcome,
    deliverables,
    decisions,
    verification,
    risks,
    nextActions,
    prior: singleTag(event, "prior"),
  };
}

export function reduceWorkReports(
  events: readonly RelayEvent[],
  channelId: string,
  rootId: string,
): WorkReport | null {
  return (
    events
      .map((event) => parseWorkReport(event, channelId, rootId))
      .filter((report): report is WorkReport => report !== null)
      .sort(
        (a, b) =>
          a.createdAt - b.createdAt ||
          (a.eventId < b.eventId ? -1 : a.eventId > b.eventId ? 1 : 0),
      )
      .at(-1) ?? null
  );
}
