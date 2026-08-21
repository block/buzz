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

function frequentCronDescription(cron: string): string | null {
  const fields = cron.trim().split(/\s+/);
  if (fields.length !== 5) return null;
  const [minute, hour] = fields;
  if (hour !== "*") return null;

  if (minute === "*") {
    return "It is scheduled to run every minute. Review the schedule before turning it on.";
  }
  const steppedMinute = /^\*\/(\d+)$/.exec(minute);
  if (steppedMinute) {
    const minutes = Number(steppedMinute[1]);
    if (minutes >= 1 && minutes <= 60) {
      return `It is scheduled to run every ${minutes} minute${minutes === 1 ? "" : "s"}. Review the schedule before turning it on.`;
    }
    return null;
  }
  if (/^\d+$/.test(minute)) {
    return "It is scheduled to run every hour. Review the schedule before turning it on.";
  }
  if (minute.includes(",") || minute.includes("-")) {
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
