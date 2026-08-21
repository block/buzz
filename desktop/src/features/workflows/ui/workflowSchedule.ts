import type { TriggerConfig } from "./workflowFormTypes";

export const SCHEDULE_FREQUENCIES = [
  "every_15_minutes",
  "every_30_minutes",
  "hourly",
  "daily",
  "weekly",
  "monthly",
  "custom_cron",
] as const;

export type ScheduleFrequency = (typeof SCHEDULE_FREQUENCIES)[number];
type ScheduleFormFrequency = ScheduleFrequency | "custom_interval";

export type ScheduleFormState = {
  customCron: string;
  customInterval: string;
  frequency: ScheduleFormFrequency;
  monthDay: string;
  time: string;
  weekday: string;
};

export const SCHEDULE_FREQUENCY_LABELS: Record<ScheduleFrequency, string> = {
  every_15_minutes: "Every 15 minutes",
  every_30_minutes: "Every 30 minutes",
  hourly: "Every hour",
  daily: "Daily",
  weekly: "Weekly",
  monthly: "Monthly",
  custom_cron: "Custom cron",
};

const INTERVAL_FREQUENCIES = {
  "15m": "every_15_minutes",
  "30m": "every_30_minutes",
  "1h": "hourly",
} as const;

const FREQUENCY_INTERVALS: Partial<Record<ScheduleFrequency, string>> = {
  every_15_minutes: "15m",
  every_30_minutes: "30m",
  hourly: "1h",
};

const DEFAULT_TIME = "09:00";
const DEFAULT_WEEKDAY = "1";
const DEFAULT_MONTH_DAY = "1";

type ParsedCommonCron = {
  frequency: "daily" | "weekly" | "monthly";
  monthDay?: string;
  time: string;
  weekday?: string;
};

export function scheduleWeekdaysFromCronField(field: string): string[] {
  const weekdays = new Set<number>();
  for (const segment of field.split(",")) {
    if (/^[0-6]$/.test(segment)) {
      weekdays.add(Number(segment));
      continue;
    }

    const range = /^([0-6])-([0-6])$/.exec(segment);
    if (!range || Number(range[1]) > Number(range[2])) return [];
    for (let day = Number(range[1]); day <= Number(range[2]); day += 1) {
      weekdays.add(day);
    }
  }
  return [...weekdays].sort((left, right) => left - right).map(String);
}

function parseCommonCron(cron: string): ParsedCommonCron | null {
  const fields = cron.trim().split(/\s+/);
  if (fields.length !== 5) return null;

  const [minute, hour, monthDay, month, weekday] = fields;
  const parsedMinute = Number(minute);
  const parsedHour = Number(hour);
  if (
    !/^\d+$/.test(minute) ||
    !/^\d+$/.test(hour) ||
    parsedMinute < 0 ||
    parsedMinute > 59 ||
    parsedHour < 0 ||
    parsedHour > 23 ||
    month !== "*"
  ) {
    return null;
  }

  const time = `${String(parsedHour).padStart(2, "0")}:${String(parsedMinute).padStart(2, "0")}`;
  if (monthDay === "*" && weekday === "*") {
    return { frequency: "daily", time };
  }
  if (monthDay === "*" && scheduleWeekdaysFromCronField(weekday).length > 0) {
    return { frequency: "weekly", time, weekday };
  }
  if (weekday === "*" && /^(?:[1-9]|[12]\d|3[01])$/.test(monthDay)) {
    return { frequency: "monthly", monthDay, time };
  }
  return null;
}

export function scheduleFormFromTrigger(
  trigger: TriggerConfig,
): ScheduleFormState {
  const interval = trigger.interval;
  if (interval) {
    const normalizedInterval = interval.trim();
    const frequency =
      INTERVAL_FREQUENCIES[
        normalizedInterval as keyof typeof INTERVAL_FREQUENCIES
      ];
    return {
      customCron: "",
      customInterval: frequency ? "" : interval,
      frequency: frequency ?? "custom_interval",
      monthDay: DEFAULT_MONTH_DAY,
      time: DEFAULT_TIME,
      weekday: DEFAULT_WEEKDAY,
    };
  }

  const cron = trigger.cron ?? "";
  const commonCron = cron ? parseCommonCron(cron) : null;
  return {
    customCron: commonCron ? "" : cron,
    customInterval: "",
    frequency: commonCron?.frequency ?? (cron ? "custom_cron" : "daily"),
    monthDay: commonCron?.monthDay ?? DEFAULT_MONTH_DAY,
    time: commonCron?.time ?? DEFAULT_TIME,
    weekday: commonCron?.weekday ?? DEFAULT_WEEKDAY,
  };
}

function cronTrigger(cron: string): TriggerConfig {
  return { on: "schedule", cron };
}

function intervalTrigger(interval: string): TriggerConfig {
  return { on: "schedule", interval };
}

function cronTime(time: string): { hour: string; minute: string } {
  const match = /^(\d{2}):(\d{2})$/.exec(time);
  if (!match) return { hour: "9", minute: "0" };
  return {
    hour: String(Number(match[1])),
    minute: String(Number(match[2])),
  };
}

export function scheduleTriggerFromForm(
  form: ScheduleFormState,
): TriggerConfig {
  const interval = FREQUENCY_INTERVALS[form.frequency as ScheduleFrequency];
  if (interval) return intervalTrigger(interval);
  if (form.frequency === "custom_interval") {
    return intervalTrigger(form.customInterval);
  }
  if (form.frequency === "custom_cron") {
    return cronTrigger(form.customCron);
  }

  const { hour, minute } = cronTime(form.time);
  switch (form.frequency) {
    case "weekly":
      return cronTrigger(`${minute} ${hour} * * ${form.weekday}`);
    case "monthly":
      return cronTrigger(`${minute} ${hour} ${form.monthDay} * *`);
    default:
      return cronTrigger(`${minute} ${hour} * * *`);
  }
}

export function defaultScheduleTrigger(): TriggerConfig {
  return scheduleTriggerFromForm(scheduleFormFromTrigger({ on: "schedule" }));
}
