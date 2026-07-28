import type { BattleRhythmEvent } from "./contracts";
import {
  localDateTimeToRfc3339,
  overlapsRange,
  type DateRange,
} from "./dateRange";

function wallTime(value: string, timeZone: string): string {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(value));
  const part = (type: string) =>
    parts.find((item) => item.type === type)?.value ?? "";
  return `${part("year")}-${part("month")}-${part("day")}T${part("hour")}:${part("minute")}`;
}

function advanceWall(
  value: string,
  frequency: "daily" | "weekly" | "monthly",
  interval: number,
): string {
  const [date, time] = value.split("T");
  const [year, month, day] = date.split("-").map(Number);
  const next = new Date(Date.UTC(year, month - 1, day));
  if (frequency === "monthly") next.setUTCMonth(next.getUTCMonth() + interval);
  else
    next.setUTCDate(
      next.getUTCDate() + interval * (frequency === "weekly" ? 7 : 1),
    );
  return `${next.toISOString().slice(0, 10)}T${time}`;
}

export function expandRecurringEvents(
  events: readonly BattleRhythmEvent[],
  range: DateRange,
): readonly BattleRhythmEvent[] {
  const expanded: BattleRhythmEvent[] = [];
  for (const event of events) {
    if (!event.recurrence) {
      if (overlapsRange(event.start, event.end, range)) expanded.push(event);
      continue;
    }
    const duration = Date.parse(event.end) - Date.parse(event.start);
    const excluded = new Set(event.excludedOccurrenceStarts.map(Date.parse));
    let wall = wallTime(event.start, event.timeZone);
    for (let index = 0; index < 1000; index += 1) {
      const start = localDateTimeToRfc3339(wall, event.timeZone);
      const startMs = Date.parse(start);
      if (
        event.recurrence.until !== null &&
        startMs > Date.parse(event.recurrence.until)
      )
        break;
      if (startMs >= Date.parse(range.end)) break;
      const end = new Date(startMs + duration).toISOString();
      if (!excluded.has(startMs) && overlapsRange(start, end, range)) {
        expanded.push(
          Object.freeze({
            ...event,
            id: index === 0 ? event.id : `${event.id}:${start}`,
            start,
            end,
          }),
        );
      }
      wall = advanceWall(
        wall,
        event.recurrence.frequency,
        event.recurrence.interval,
      );
    }
  }
  return Object.freeze(
    expanded.sort(
      (left, right) => Date.parse(left.start) - Date.parse(right.start),
    ),
  );
}
