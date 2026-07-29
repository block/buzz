import type {
  PlanningPlaybookV1,
  PlaybookTaskTemplateV1,
} from "./extendedContracts";

export type PlaybookScheduleContext = Readonly<{
  anchorDate: string;
  anchorTime: string;
  routine: "alongside" | "atSea";
  timeZone: string;
}>;

export type ScheduledPlaybookTask = Readonly<{
  template: PlaybookTaskTemplateV1;
  plannedStart: string;
  plannedStartTime: string;
  dueDate: string;
  dueTime: string;
  timeZone: string;
  dependencyIds: readonly string[];
  assumptions: readonly string[];
}>;

function minutes(date: Date) {
  return date.getUTCHours() * 60 + date.getUTCMinutes();
}

function setMinutes(date: Date, value: number) {
  const result = new Date(date);
  result.setUTCHours(Math.floor(value / 60), value % 60, 0, 0);
  return result;
}

function previousDayEnd(date: Date) {
  const result = new Date(date);
  result.setUTCHours(0, 0, 0, 0);
  return new Date(result.getTime() - 1);
}

function availability(
  date: Date,
  routine: PlaybookScheduleContext["routine"],
): readonly [number, number] | null {
  const day = date.getUTCDay();
  if (routine === "alongside")
    return day >= 1 && day <= 5 ? [8 * 60, 16 * 60] : null;
  return day === 0 ? [12 * 60, 24 * 60] : [0, 24 * 60];
}

function normalizeBackward(
  date: Date,
  routine: PlaybookScheduleContext["routine"],
): Date {
  let cursor = new Date(date);
  for (let count = 0; count < 370; count += 1) {
    const interval = availability(cursor, routine);
    if (interval) {
      const value = minutes(cursor);
      if (value > interval[0] && value <= interval[1]) return cursor;
      if (value > interval[1]) return setMinutes(cursor, interval[1]);
    }
    cursor = previousDayEnd(cursor);
  }
  throw new Error(
    "No ship routine availability within the scheduling horizon.",
  );
}

function subtractWorkingMinutes(
  date: Date,
  amount: number,
  routine: PlaybookScheduleContext["routine"],
): Date {
  let cursor = normalizeBackward(date, routine);
  let remaining = amount;
  while (remaining > 0) {
    const interval = availability(cursor, routine);
    if (!interval) {
      cursor = normalizeBackward(previousDayEnd(cursor), routine);
      continue;
    }
    const available = minutes(cursor) - interval[0];
    if (available >= remaining)
      return new Date(cursor.getTime() - remaining * 60_000);
    remaining -= Math.max(0, available);
    cursor = normalizeBackward(previousDayEnd(cursor), routine);
  }
  return cursor;
}

function stamp(date: Date) {
  return {
    date: date.toISOString().slice(0, 10),
    time: date.toISOString().slice(11, 16),
  };
}

export function schedulePlaybook(
  playbook: PlanningPlaybookV1,
  context: PlaybookScheduleContext,
): readonly ScheduledPlaybookTask[] {
  const anchor = new Date(
    `${context.anchorDate}T${context.anchorTime}:00.000Z`,
  );
  if (Number.isNaN(anchor.getTime()))
    throw new Error("Playbook anchor is invalid.");
  return Object.freeze(
    playbook.taskTemplates.map((template) => {
      if (template.timing !== "before")
        throw new Error("After-anchor playbooks are not available in V1.");
      const desired = new Date(
        anchor.getTime() - template.offsetMinutes * 60_000,
      );
      const due = normalizeBackward(desired, context.routine);
      const start = subtractWorkingMinutes(
        due,
        template.durationMinutes,
        context.routine,
      );
      const dueStamp = stamp(due);
      const startStamp = stamp(start);
      return Object.freeze({
        template,
        plannedStart: startStamp.date,
        plannedStartTime: startStamp.time,
        dueDate: dueStamp.date,
        dueTime: dueStamp.time,
        timeZone: context.timeZone,
        dependencyIds: Object.freeze([...template.dependencyIds]),
        assumptions: Object.freeze([
          context.routine === "alongside"
            ? "Alongside routine: Monday-Friday 0800-1600."
            : "At-sea routine: continuous Monday-Saturday; Sunday from 1200.",
        ]),
      });
    }),
  );
}
