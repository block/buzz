import type { Workflow } from "@/shared/api/types";
import { TRIGGER_LABELS } from "./workflowFormTypes";
import type { TriggerType } from "./workflowFormTypes";

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

export function getWorkflowEnabled(
  definition: Record<string, unknown>,
): boolean {
  return definition.enabled !== false;
}

export function getWorkflowDisplayStatus(
  workflow: Workflow,
): Workflow["status"] | "disabled" {
  if (workflow.status !== "active") {
    return workflow.status;
  }

  return getWorkflowEnabled(workflow.definition) ? workflow.status : "disabled";
}

export function getWorkflowDescription(
  definition: Record<string, unknown>,
): string | null {
  const description = definition.description;
  return typeof description === "string" && description.trim().length > 0
    ? description.trim()
    : null;
}

export function getWorkflowTriggerSummary(
  definition: Record<string, unknown>,
): string | null {
  const trigger = asRecord(definition.trigger);
  if (!trigger) return null;

  const on = trigger.on;
  if (typeof on !== "string") return null;

  const label = TRIGGER_LABELS[on as TriggerType] ?? on;
  switch (on) {
    case "message_posted":
    case "diff_posted":
      return typeof trigger.filter === "string" &&
        trigger.filter.trim().length > 0
        ? `${label} · ${trigger.filter}`
        : label;
    case "reaction_added":
      return typeof trigger.emoji === "string" &&
        trigger.emoji.trim().length > 0
        ? `${label} · ${trigger.emoji}`
        : label;
    case "schedule":
      if (typeof trigger.cron === "string" && trigger.cron.trim().length > 0) {
        return `${label} · ${trigger.cron}`;
      }
      if (
        typeof trigger.interval === "string" &&
        trigger.interval.trim().length > 0
      ) {
        return `${label} · ${trigger.interval}`;
      }
      return label;
    default:
      return label;
  }
}
