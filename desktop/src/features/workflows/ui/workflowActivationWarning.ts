import { parse as yamlParse } from "yaml";

import {
  formatDurationSecondsVerbose,
  parseDurationSeconds,
} from "./workflowDuration";

type WorkflowActivationWarning = {
  description: string;
  title: string;
};

const FREQUENT_SCHEDULE_THRESHOLD_SECONDS = 60 * 60;

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function frequentIntervalDescription(interval: string): string | null {
  const seconds = parseDurationSeconds(interval);
  if (
    seconds === null ||
    seconds <= 0 ||
    seconds > FREQUENT_SCHEDULE_THRESHOLD_SECONDS
  ) {
    return null;
  }
  return `It is scheduled to run every ${formatDurationSecondsVerbose(seconds)}. Review the schedule before turning it on.`;
}

function normalizedCronFields(cron: string): string[] | null {
  const fields = cron.trim().split(/\s+/);
  if (fields.length === 5) return ["0", ...fields, "*"];
  if (fields.length === 6) return [...fields, "*"];
  return fields.length === 7 ? fields : null;
}

function repeatedFieldCount(field: string, maximum: number): number | null {
  if (field === "*") return maximum + 1;
  const step = /^\*\/(\d+)$/.exec(field);
  if (step) {
    const size = Number(step[1]);
    return size >= 1 && size <= maximum + 1
      ? Math.ceil((maximum + 1) / size)
      : null;
  }
  if (field.includes(",") || field.includes("-")) return 2;
  return /^\d+$/.test(field) ? 1 : null;
}

/**
 * Reports whether an hour field selects every hour of the day, so schedules
 * written as `*`, `*\/1`, `0-23`, or an equivalent list still classify as
 * hourly. Unrecognized fields are treated as not matching every hour.
 */
function matchesEveryHour(field: string): boolean {
  const HOURS = 24;
  const selected = new Set<number>();
  for (const segment of field.split(",")) {
    const [base, step] = segment.split("/");
    if (step !== undefined && (!/^\d+$/.test(step) || Number(step) < 1)) {
      return false;
    }
    const size = step === undefined ? 1 : Number(step);
    let start: number, end: number;
    if (base === "*") {
      start = 0;
      end = HOURS - 1;
    } else {
      const bounds = base.split("-");
      if (bounds.length > 2 || bounds.some((part) => !/^\d+$/.test(part))) {
        return false;
      }
      start = Number(bounds[0]);
      end = bounds.length === 2 ? Number(bounds[1]) : start;
      if (start > end || end >= HOURS) return false;
      // A bare hour selects only itself; a stepped one runs to end of day.
      if (bounds.length === 1) {
        if (step === undefined) {
          selected.add(start);
          continue;
        }
        end = HOURS - 1;
      }
    }
    for (let hour = start; hour <= end; hour += size) selected.add(hour);
  }
  return selected.size === HOURS;
}

function frequentCronDescription(cron: string): string | null {
  const fields = normalizedCronFields(cron);
  if (!fields) return null;
  const [second, minute, hour] = fields;
  const secondRuns = repeatedFieldCount(second, 59);
  const minuteRuns = repeatedFieldCount(minute, 59);
  if (secondRuns === null || minuteRuns === null) return null;
  if (!matchesEveryHour(hour)) return null;

  if (secondRuns > 1) {
    return "It is scheduled to run multiple times a minute. Review the schedule before turning it on.";
  }
  if (minute === "*") {
    return "It is scheduled to run every minute. Review the schedule before turning it on.";
  }
  const steppedMinute = /^\*\/(\d+)$/.exec(minute);
  if (steppedMinute) {
    const minutes = Number(steppedMinute[1]);
    return `It is scheduled to run every ${minutes} minute${minutes === 1 ? "" : "s"}. Review the schedule before turning it on.`;
  }
  if (minuteRuns === 1) {
    return "It is scheduled to run every hour. Review the schedule before turning it on.";
  }
  if (minuteRuns > 1) {
    return "It is scheduled to run multiple times an hour. Review the schedule before turning it on.";
  }
  return null;
}

export function getWorkflowActivationWarning(
  yaml: string,
): WorkflowActivationWarning | null {
  let definition: Record<string, unknown> | null;
  try {
    definition = asRecord(yamlParse(yaml));
  } catch {
    return null;
  }
  const trigger = asRecord(definition?.trigger);
  const triggerType = nonEmptyString(trigger?.on);

  if (triggerType === "message_posted" && !nonEmptyString(trigger?.filter)) {
    return {
      description:
        "It will run for every new message in this channel. Review the trigger before turning it on.",
      title: "This workflow may run often",
    };
  }

  if (triggerType === "schedule") {
    const interval = nonEmptyString(trigger?.interval);
    const cron = nonEmptyString(trigger?.cron);
    const description = interval
      ? frequentIntervalDescription(interval)
      : cron
        ? frequentCronDescription(cron)
        : null;
    if (description) {
      return { description, title: "This workflow may run often" };
    }
  }

  return null;
}
