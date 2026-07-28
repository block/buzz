import {
  readAppleInputs,
  type AppleInputPermission,
  type AppleInputResponse,
} from "@/shared/api/tauriAppleInputs";
import type { DateRange } from "../domain/dateRange";
import type { BattleRhythmEvent } from "../domain/contracts";

export type ApplePublicationState =
  | "published"
  | "changes_pending"
  | "permission_required"
  | "unavailable";

export type ApplePublicationStatus = Readonly<{
  state: ApplePublicationState;
  permission: AppleInputPermission;
  calendarIdentifier: string | null;
  created: number;
  updated: number;
  deleted: number;
  unchanged: number;
  error: string | null;
}>;

function bounded(value: string | null, maximum: number): string | null {
  if (!value) return null;
  return value.slice(0, maximum);
}

export function projectBattleRhythmToApple(
  events: readonly BattleRhythmEvent[],
) {
  return events
    .filter((event) => event.status === "approved")
    .map((event) => {
      const notes = [
        event.description,
        event.remarks,
        event.responsibleOwner
          ? `Responsible: ${event.responsibleOwner}`
          : null,
        event.participants.length > 0
          ? `Participants: ${event.participants.join(", ")}`
          : null,
      ]
        .filter((value): value is string => Boolean(value))
        .join("\n");
      return Object.freeze({
        external_id: `battle-rhythm:${event.id}`,
        title: event.title.slice(0, 512),
        start: event.start,
        end: event.end,
        is_all_day: event.allDay,
        location: bounded(event.location, 1024),
        notes: bounded(notes, 4096),
      });
    });
}

function count(value: string | undefined): number {
  if (!value || !/^(0|[1-9]\d{0,6})$/.test(value))
    throw new Error("Apple Calendar returned an invalid publication result.");
  return Number(value);
}

export function parseApplePublicationStatus(
  response: AppleInputResponse,
): ApplePublicationStatus {
  if (response.source !== "calendar")
    throw new Error("Apple Calendar returned an invalid publication result.");
  const fields = response.records[0]?.fields;
  if (!fields) {
    return Object.freeze({
      state:
        response.permission === "not_determined" ||
        response.permission === "denied" ||
        response.permission === "restricted"
          ? "permission_required"
          : "unavailable",
      permission: response.permission,
      calendarIdentifier: null,
      created: 0,
      updated: 0,
      deleted: 0,
      unchanged: 0,
      error: response.error,
    });
  }
  const allowed = new Set([
    "calendar_identifier",
    "created",
    "updated",
    "deleted",
    "unchanged",
  ]);
  if (Object.keys(fields).some((key) => !allowed.has(key)))
    throw new Error("Apple Calendar returned an invalid publication result.");
  const result = {
    permission: response.permission,
    calendarIdentifier: fields.calendar_identifier ?? null,
    created: count(fields.created),
    updated: count(fields.updated),
    deleted: count(fields.deleted),
    unchanged: count(fields.unchanged),
    error: response.error,
  };
  return Object.freeze({
    ...result,
    state:
      response.permission !== "authorized"
        ? "permission_required"
        : response.error
          ? "unavailable"
          : "published",
  });
}

export async function publishBattleRhythmToApple(
  events: readonly BattleRhythmEvent[],
  coverage: DateRange,
): Promise<ApplePublicationStatus> {
  const response = await readAppleInputs({
    operation: "reconcile_calendar",
    arguments: {
      coverage_start: coverage.start,
      coverage_end: coverage.end,
      projections: projectBattleRhythmToApple(events),
    },
  });
  return parseApplePublicationStatus(response);
}
