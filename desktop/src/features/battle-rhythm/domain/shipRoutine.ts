import type { BattleRhythmEvent, BattleRhythmSource } from "./contracts";

export type ShipRoutine = "alongside" | "atSea";

export type ShipRoutinePeriod = Readonly<{
  start: string;
  end: string;
  routine: ShipRoutine;
  timeZone: string;
  assumed: boolean;
  sourceEventId: string | null;
  findings: readonly string[];
}>;

const DEFAULT_TIME_ZONE = "Australia/Sydney";

function validTimeZone(value: string): boolean {
  try {
    new Intl.DateTimeFormat("en-AU", { timeZone: value }).format();
    return true;
  } catch {
    return false;
  }
}

function sourceIsApproved(
  event: BattleRhythmEvent,
  sources: ReadonlyMap<string, BattleRhythmSource>,
): boolean {
  if (event.status !== "approved") return false;
  if (event.ownership.kind === "manual") return true;
  const source = sources.get(event.ownership.sourceId);
  return source?.status === "approved";
}

export function deriveShipRoutinePeriods(
  sources: readonly BattleRhythmSource[],
  events: readonly BattleRhythmEvent[],
  range: Readonly<{ start: string; end: string }>,
): readonly ShipRoutinePeriod[] {
  const sourceById = new Map(sources.map((source) => [source.id, source]));
  const relevant = events
    .filter((event) => sourceIsApproved(event, sourceById))
    .filter((event) =>
      ["routine_alongside", "routine_at_sea", "timezone_change"].includes(
        event.type,
      ),
    );
  const invalidZones = relevant
    .filter(
      (event) =>
        event.type === "timezone_change" &&
        (!event.remarks || !validTimeZone(event.remarks.trim())),
    )
    .map(
      (event) =>
        `Ignored invalid Ship Time zone ${event.remarks ?? "(missing)"} from ${event.title}.`,
    );
  const boundaries = new Set<number>([
    Date.parse(range.start),
    Date.parse(range.end),
  ]);
  for (const event of relevant) {
    boundaries.add(Date.parse(event.start));
    if (event.type !== "timezone_change") boundaries.add(Date.parse(event.end));
  }
  const ordered = [...boundaries]
    .filter(
      (value) =>
        Number.isFinite(value) &&
        value >= Date.parse(range.start) &&
        value <= Date.parse(range.end),
    )
    .sort((left, right) => left - right);
  let lastRoutine: ShipRoutine = "alongside";
  const result: ShipRoutinePeriod[] = [];
  for (let index = 0; index < ordered.length - 1; index += 1) {
    const start = ordered[index];
    const end = ordered[index + 1];
    if (start === undefined || end === undefined || start >= end) continue;
    const activeRoutine = relevant
      .filter(
        (event) =>
          (event.type === "routine_alongside" ||
            event.type === "routine_at_sea") &&
          Date.parse(event.start) <= start &&
          Date.parse(event.end) > start,
      )
      .sort(
        (left, right) => Date.parse(right.start) - Date.parse(left.start),
      )[0];
    if (activeRoutine)
      lastRoutine =
        activeRoutine.type === "routine_at_sea" ? "atSea" : "alongside";
    const zoneEvent = relevant
      .filter(
        (event) =>
          event.type === "timezone_change" &&
          Date.parse(event.start) <= start &&
          Boolean(event.remarks && validTimeZone(event.remarks.trim())),
      )
      .sort(
        (left, right) => Date.parse(right.start) - Date.parse(left.start),
      )[0];
    result.push(
      Object.freeze({
        start: new Date(start).toISOString(),
        end: new Date(end).toISOString(),
        routine: lastRoutine,
        timeZone: zoneEvent?.remarks?.trim() ?? DEFAULT_TIME_ZONE,
        assumed: !activeRoutine,
        sourceEventId: activeRoutine?.id ?? null,
        findings: Object.freeze([...invalidZones]),
      }),
    );
  }
  return Object.freeze(result);
}

export function shipStateAt(
  periods: readonly ShipRoutinePeriod[],
  timestamp: string,
): ShipRoutinePeriod {
  const instant = Date.parse(timestamp);
  const match = periods.find(
    (period) =>
      Date.parse(period.start) <= instant && Date.parse(period.end) > instant,
  );
  return (
    match ??
    Object.freeze({
      start: timestamp,
      end: timestamp,
      routine: "alongside",
      timeZone: DEFAULT_TIME_ZONE,
      assumed: true,
      sourceEventId: null,
      findings: Object.freeze(["No approved FAS routine covers this time."]),
    })
  );
}
