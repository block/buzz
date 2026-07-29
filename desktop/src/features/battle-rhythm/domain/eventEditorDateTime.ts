import { addDays, localDateTimeToRfc3339 } from "./dateRange";

function localParts(date: Date, timeZone: string): Record<string, string> {
  return Object.fromEntries(
    new Intl.DateTimeFormat("en-CA", {
      timeZone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    })
      .formatToParts(date)
      .map((part) => [part.type, part.value]),
  );
}

export function editorLocalDateTime(
  timestamp: string | Date,
  timeZone: string,
): string {
  const parts = localParts(
    timestamp instanceof Date ? timestamp : new Date(timestamp),
    timeZone,
  );
  return `${parts.year}-${parts.month}-${parts.day}T${parts.hour}:${parts.minute}`;
}

export function defaultEventWindow(
  timeZone: string,
  now = new Date(),
): { start: string; end: string } {
  return {
    start: editorLocalDateTime(now, timeZone),
    end: editorLocalDateTime(new Date(now.getTime() + 3_600_000), timeZone),
  };
}

export function datePart(value: string): string {
  return value.slice(0, 10);
}

export function timePart(value: string): string {
  return value.slice(11, 16);
}

export function withDate(value: string, date: string): string {
  return `${date}T${timePart(value) || "00:00"}`;
}

export function withTime(value: string, time: string): string {
  return `${datePart(value)}T${time}`;
}

function wallClockMilliseconds(value: string): number {
  return Date.parse(`${value}:00Z`);
}

export function isCompleteLocalDateTime(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}T(?:[01]\d|2[0-3]):[0-5]\d$/.test(value);
}

export function shiftEndKeepingDuration(
  previousStart: string,
  previousEnd: string,
  nextStart: string,
): string {
  const nextStartMilliseconds = wallClockMilliseconds(nextStart);
  if (!Number.isFinite(nextStartMilliseconds)) return previousEnd;

  const previousDuration =
    wallClockMilliseconds(previousEnd) - wallClockMilliseconds(previousStart);
  const duration =
    Number.isFinite(previousDuration) && previousDuration > 0
      ? previousDuration
      : 3_600_000;
  return new Date(nextStartMilliseconds + duration).toISOString().slice(0, 16);
}

export function editorEndForEvent(
  end: string,
  allDay: boolean,
  timeZone: string,
): string {
  const local = editorLocalDateTime(end, timeZone);
  if (!allDay || timePart(local) !== "00:00") return local;
  return `${addDays(datePart(local), -1)}T00:00`;
}

export function eventWindowForSave(
  start: string,
  end: string,
  allDay: boolean,
  timeZone: string,
): { start: string; end: string } {
  if (allDay) {
    const startDay = datePart(start);
    const endDay = datePart(end);
    if (endDay < startDay) {
      throw new Error("End date must be on or after the start date.");
    }
    return {
      start: localDateTimeToRfc3339(`${startDay}T00:00`, timeZone),
      end: localDateTimeToRfc3339(`${addDays(endDay, 1)}T00:00`, timeZone),
    };
  }

  const savedStart = localDateTimeToRfc3339(start, timeZone);
  const savedEnd = localDateTimeToRfc3339(end, timeZone);
  if (Date.parse(savedEnd) <= Date.parse(savedStart)) {
    throw new Error("End time must be after the start time.");
  }
  return { start: savedStart, end: savedEnd };
}
