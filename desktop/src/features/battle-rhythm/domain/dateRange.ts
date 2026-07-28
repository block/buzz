export type DateRange = Readonly<{ start: string; end: string }>;

function dateParts(date: Date, timeZone: string) {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  return Object.fromEntries(parts.map((part) => [part.type, part.value]));
}

function offsetFor(date: Date, timeZone: string): string {
  const part = new Intl.DateTimeFormat("en-US", {
    timeZone,
    timeZoneName: "longOffset",
  })
    .formatToParts(date)
    .find((item) => item.type === "timeZoneName")?.value;
  return part?.replace("GMT", "") || "+00:00";
}

export function localDateTimeToRfc3339(
  value: string,
  timeZone: string,
): string {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value)) {
    throw new Error("Local date-time must use YYYY-MM-DDTHH:mm");
  }
  const guessedInstant = new Date(`${value}:00Z`);
  if (Number.isNaN(guessedInstant.getTime())) {
    throw new Error("Local date-time is invalid");
  }
  return `${value}:00${offsetFor(guessedInstant, timeZone)}`;
}

function localMidnight(day: string, timeZone: string): string {
  const [year, month, date] = day.split("-").map(Number);
  const guessed = new Date(Date.UTC(year, month - 1, date));
  return `${day}T00:00:00${offsetFor(guessed, timeZone)}`;
}

function addDays(day: string, amount: number): string {
  const [year, month, date] = day.split("-").map(Number);
  const next = new Date(Date.UTC(year, month - 1, date + amount));
  return next.toISOString().slice(0, 10);
}

function weekday(day: string): number {
  const [year, month, date] = day.split("-").map(Number);
  return (new Date(Date.UTC(year, month - 1, date)).getUTCDay() + 6) % 7;
}

export function getWeekRange(day: string, timeZone: string): DateRange {
  const startDay = addDays(day, -weekday(day));
  const endDay = addDays(startDay, 7);
  return {
    start: localMidnight(startDay, timeZone),
    end: localMidnight(endDay, timeZone),
  };
}

export function getYearRange(
  day: string,
  timeZone: string,
  months = 12,
): DateRange {
  const year = day.slice(0, 4);
  return {
    start: localMidnight(`${year}-01-01`, timeZone),
    end: localMidnight(
      `${Number(year) + Math.ceil(months / 12)}-01-01`,
      timeZone,
    ),
  };
}

export function getMonthCells(day: string, timeZone: string): string[] {
  void timeZone;
  const monthStart = `${day.slice(0, 7)}-01`;
  const [year, month] = monthStart.split("-").map(Number);
  const nextMonth = new Date(Date.UTC(year, month, 1));
  const totalDays = Math.round(
    (nextMonth.getTime() - Date.UTC(year, month - 1, 1)) / 86_400_000,
  );
  const leading = weekday(monthStart);
  const cellCount = Math.ceil((leading + totalDays) / 7) * 7;
  return Array.from({ length: cellCount }, (_, index) =>
    addDays(monthStart, index - leading),
  );
}

export function dayInTimeZone(date: Date, timeZone: string): string {
  const parts = dateParts(date, timeZone);
  return `${parts.year}-${parts.month}-${parts.day}`;
}

export function overlapsRange(
  start: string,
  end: string,
  range: DateRange,
): boolean {
  return (
    Date.parse(start) < Date.parse(range.end) &&
    Date.parse(end) > Date.parse(range.start)
  );
}
