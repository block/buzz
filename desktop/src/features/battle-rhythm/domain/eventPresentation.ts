import type { BattleRhythmEvent } from "./contracts";
import { addDays, type DateRange, overlapsCalendarDay } from "./dateRange";

export type ProgramEventTone = "sea" | "port" | "neutral";

export function programEventTone(
  event: Pick<BattleRhythmEvent, "allDay" | "location">,
): ProgramEventTone {
  if (!event.allDay) return "neutral";
  const location = event.location?.trim();
  if (!location) return "neutral";
  return /\bsea\b/i.test(location) ? "sea" : "port";
}

export function weekAllDayPlacement(
  event: Pick<BattleRhythmEvent, "allDay" | "start" | "end">,
  range: DateRange,
  timeZone: string,
): Readonly<{ startColumn: number; span: number }> | null {
  if (!event.allDay) return null;

  const weekStart = range.start.slice(0, 10);
  const columns = Array.from({ length: 7 }, (_, offset) => offset).filter(
    (offset) =>
      overlapsCalendarDay(
        event.start,
        event.end,
        addDays(weekStart, offset),
        timeZone,
      ),
  );
  if (columns.length === 0) return null;

  return {
    startColumn: columns[0] + 1,
    span: columns.length,
  };
}
