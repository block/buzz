import { getMonthCells, getWeekRange } from "./dateRange";

export type CalendarView = "Year" | "Month" | "Week" | "Day";

export type CalendarMonth = Readonly<{
  month: number;
  label: string;
  cells: readonly string[];
}>;

function calendarDate(day: string): Date {
  return new Date(`${day}T12:00:00Z`);
}

function formatDay(
  day: string,
  timeZone: string,
  options: Intl.DateTimeFormatOptions,
): string {
  return new Intl.DateTimeFormat("en-AU", {
    ...options,
    timeZone,
  }).format(calendarDate(day));
}

export function calendarHeading(
  view: CalendarView,
  day: string,
  timeZone: string,
): string {
  if (view === "Day") {
    const formatted = formatDay(day, timeZone, {
      weekday: "long",
      day: "numeric",
      month: "long",
      year: "numeric",
    });
    const [weekday] = formatted.split(" ", 1);
    return `${weekday}, ${formatted.slice(weekday.length + 1)}`;
  }
  if (view === "Month") {
    return formatDay(day, timeZone, {
      month: "long",
      year: "numeric",
    });
  }
  if (view === "Year") {
    return day.slice(0, 4);
  }

  const range = getWeekRange(day, timeZone);
  const start = range.start.slice(0, 10);
  const endDate = calendarDate(range.end.slice(0, 10));
  endDate.setUTCDate(endDate.getUTCDate() - 1);
  const end = endDate.toISOString().slice(0, 10);
  const startYear = start.slice(0, 4);
  const endYear = end.slice(0, 4);
  const startMonth = start.slice(5, 7);
  const endMonth = end.slice(5, 7);

  const startText = formatDay(start, timeZone, {
    day: "numeric",
    month: "long",
    ...(startYear !== endYear ? { year: "numeric" } : {}),
  });
  const endText = formatDay(end, timeZone, {
    day: "numeric",
    ...(startMonth !== endMonth ? { month: "long" } : {}),
    year: "numeric",
  });
  return `${startText} – ${endText}`;
}

export function weekDayHeading(day: string, timeZone: string): string {
  const parts = new Intl.DateTimeFormat("en-AU", {
    weekday: "short",
    day: "numeric",
    month: "short",
    timeZone,
  }).formatToParts(calendarDate(day));
  const value = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((part) => part.type === type)?.value ?? "";
  return `${value("weekday")} ${value("day")} ${value("month").slice(0, 3)}`.toUpperCase();
}

export function formatShipTime(timestamp: string, timeZone: string): string {
  return new Intl.DateTimeFormat("en-AU", {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZone,
  }).format(new Date(timestamp));
}

export function monthGrid(
  day: string,
  timeZone: string,
): readonly CalendarMonth[] {
  const year = Number(day.slice(0, 4));
  return Array.from({ length: 12 }, (_, index) => {
    const month = index + 1;
    const monthDay = `${year}-${String(month).padStart(2, "0")}-01`;
    return {
      month,
      label: formatDay(monthDay, timeZone, { month: "long" }),
      cells: getMonthCells(monthDay, timeZone),
    };
  });
}
