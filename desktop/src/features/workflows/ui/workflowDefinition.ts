import type { Workflow } from "@/shared/api/types";
import { i18n } from "@/i18n";
import {
  scheduleFrequencyLabel,
  scheduleFormFromTrigger,
} from "./workflowSchedule";
import {
  actionTypeLabel,
  isKnownActionType,
  TRIGGER_TYPES,
  triggerTypeLabel,
} from "./workflowFormTypes";
import type {
  ActionType,
  StepFormState,
  TriggerConfig,
  TriggerType,
} from "./workflowFormTypes";
import { workflowStepDescription } from "./workflowStepDescription";
import { parseConditionExpressions } from "./workflowConditionExpression";
import { workflowTriggerDescription } from "./workflowTriggerDescription";

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function humanizeIdentifier(value: string): string {
  return value.replaceAll("_", " ").replace(/\s+/g, " ").trim();
}

function getWorkflowSteps(
  definition: Record<string, unknown>,
): Record<string, unknown>[] {
  return Array.isArray(definition.steps)
    ? definition.steps.map(asRecord).filter((step) => step !== null)
    : [];
}

export function getWorkflowTriggerType(
  definition: Record<string, unknown>,
): string | null {
  return nonEmptyString(asRecord(definition.trigger)?.on);
}

export function getWorkflowPrimaryAction(
  definition: Record<string, unknown>,
): string | null {
  return nonEmptyString(getWorkflowSteps(definition)[0]?.action);
}

export function getWorkflowPrimaryActionEmoji(
  definition: Record<string, unknown>,
): string | null {
  return getWorkflowActionTiles(definition)[0]?.emoji ?? null;
}

export function getWorkflowActionTiles(
  definition: Record<string, unknown>,
): Array<{ action: string; emoji: string | null; key: string }> {
  return getWorkflowSteps(definition).map((step, index) => ({
    action: nonEmptyString(step.action) ?? "",
    emoji: step.action === "add_reaction" ? nonEmptyString(step.emoji) : null,
    key: nonEmptyString(step.id) ?? `step-${index}`,
  }));
}

export function getWorkflowTriggerEmoji(
  definition: Record<string, unknown>,
): string | null {
  const trigger = asRecord(definition.trigger);
  if (trigger?.on !== "reaction_added") return null;

  const legacyEmoji = nonEmptyString(trigger.emoji);
  if (legacyEmoji) return legacyEmoji;

  const filter = nonEmptyString(trigger.filter);
  if (!filter) return null;
  const conditions = parseConditionExpressions(filter, "reaction_added");
  const emojiCondition = conditions?.find(
    ({ field, operator }) => field === "trigger_emoji" && operator === "equals",
  );
  return nonEmptyString(emojiCondition?.value);
}

export function getWorkflowTriggerConfig(
  definition: Record<string, unknown>,
): TriggerConfig | null {
  const trigger = asRecord(definition.trigger);
  const triggerType = nonEmptyString(trigger?.on);
  if (
    !trigger ||
    !triggerType ||
    !TRIGGER_TYPES.includes(triggerType as TriggerType)
  ) {
    return null;
  }

  return {
    on: triggerType as TriggerType,
    filter: nonEmptyString(trigger.filter) ?? undefined,
    emoji: nonEmptyString(trigger.emoji) ?? undefined,
    cron: nonEmptyString(trigger.cron) ?? undefined,
    interval: nonEmptyString(trigger.interval) ?? undefined,
  };
}

function getScheduleCardClause(trigger: Record<string, unknown>): string {
  const schedule = scheduleFormFromTrigger({
    on: "schedule",
    cron: nonEmptyString(trigger.cron) ?? undefined,
    interval: nonEmptyString(trigger.interval) ?? undefined,
  });

  switch (schedule.frequency) {
    case "daily":
      return i18n.t("workflows.card.schedule-daily", { time: schedule.time });
    case "weekly":
      return i18n.t("workflows.card.schedule-weekly", { time: schedule.time });
    case "monthly":
      return i18n.t("workflows.card.schedule-monthly", { time: schedule.time });
    case "custom_interval":
      return schedule.customInterval
        ? i18n.t("workflows.card.schedule-custom-interval", {
            interval: schedule.customInterval,
          })
        : i18n.t("workflows.card.schedule-on-a-schedule");
    case "custom_cron":
      return i18n.t("workflows.card.schedule-custom-cron");
    default:
      return scheduleFrequencyLabel(schedule.frequency);
  }
}

function getTriggerCardClause(
  definition: Record<string, unknown>,
  presentedReaction?: string,
): string {
  const trigger = asRecord(definition.trigger);
  const triggerType = nonEmptyString(trigger?.on);
  if (!trigger || !triggerType) {
    return i18n.t("workflows.card.trigger-fallback");
  }

  switch (triggerType) {
    case "message_posted":
      return nonEmptyString(trigger.filter)
        ? i18n.t("workflows.card.trigger-message-matching")
        : i18n.t("workflows.card.trigger-message");
    case "reaction_added": {
      const emoji = nonEmptyString(presentedReaction ?? trigger.emoji);
      return emoji
        ? i18n.t("workflows.card.trigger-reaction-with", { emoji })
        : i18n.t("workflows.card.trigger-reaction");
    }
    case "diff_posted":
      return nonEmptyString(trigger.filter)
        ? i18n.t("workflows.card.trigger-diff-matching")
        : i18n.t("workflows.card.trigger-diff");
    case "webhook":
      return i18n.t("workflows.card.trigger-webhook");
    case "schedule":
      return getScheduleCardClause(trigger);
    default:
      return i18n.t("workflows.card.trigger-unknown", {
        trigger: humanizeIdentifier(triggerType),
      });
  }
}

function eventDescriptionCardClause(
  description: string,
  event: "message" | "diff",
): string | null {
  const subject = event === "message" ? "Message" : "Diff";
  const eventPhrase = `${subject} posted`;
  if (!description.startsWith(eventPhrase)) return null;

  const detail = description.slice(eventPhrase.length);
  if (!detail) return `When a ${event} is posted`;
  if (detail.startsWith(" is ") || detail.startsWith(" is not ")) {
    return `When a ${event}${detail}`;
  }
  if (
    detail.startsWith(" containing ") ||
    detail.startsWith(" without ") ||
    detail.startsWith(" starting with ") ||
    detail.startsWith(" ending with ") ||
    detail === " with text"
  ) {
    return `When a ${event}${detail} is posted`;
  }
  return `When a ${event} is posted${detail}`;
}

function textDescriptionCardClause(
  description: string,
  event: "message" | "diff",
): string | null {
  const subject = event === "message" ? "Message" : "Diff";
  if (!description.startsWith(`${subject} `)) return null;
  return `When a ${event}${description.slice(subject.length)}`;
}

function postedValueCardClause(description: string): string | null {
  const postedIndex = description.indexOf(" posted");
  if (postedIndex < 1) return null;

  const value = description.slice(0, postedIndex);
  if (!value.startsWith("“") && !value.startsWith("Anything except ")) {
    return null;
  }
  const normalizedValue = value.startsWith("Anything")
    ? `anything${value.slice("Anything".length)}`
    : value;
  return `When ${normalizedValue} is posted${description.slice(postedIndex + " posted".length)}`;
}

function qualifiedPostedEventCardClause(
  description: string,
  event: "message" | "diff",
): string | null {
  const subject = event === "message" ? "Message" : "Diff";
  const postedIndex = description.indexOf(" posted");
  if (postedIndex < 1) return null;

  const suffix = description.slice(postedIndex + " posted".length);
  const otherTextPrefix = `${subject} with text other than `;
  if (description.startsWith(otherTextPrefix)) {
    const value = description.slice(otherTextPrefix.length, postedIndex);
    return `When a ${event} with text other than ${value} is posted${suffix}`;
  }

  if (description.startsWith(`${subject} with text posted`)) {
    return `When a ${event} with text is posted${suffix}`;
  }
  if (description.startsWith(`${subject} without text posted`)) {
    return `When a ${event} without text is posted${suffix}`;
  }
  return null;
}

function exactValueEventCardClause(
  description: string,
  event: "message" | "diff",
): string | null {
  const subject = event === "message" ? "Message" : "Diff";
  const prefix = `${subject} “`;
  const postedMarker = " is posted";
  if (!description.startsWith(prefix)) return null;

  const postedIndex = description.indexOf(postedMarker);
  if (postedIndex < prefix.length) return null;
  const value = description.slice(subject.length + 1, postedIndex);
  const suffix = description.slice(postedIndex + postedMarker.length);
  return event === "message"
    ? `When ${value} is posted${suffix}`
    : `When a diff ${value} is posted${suffix}`;
}

function getPresentedTriggerCardClause(description?: string): string | null {
  if (!description) return null;

  const eventClause =
    exactValueEventCardClause(description, "message") ??
    exactValueEventCardClause(description, "diff") ??
    qualifiedPostedEventCardClause(description, "message") ??
    qualifiedPostedEventCardClause(description, "diff") ??
    postedValueCardClause(description) ??
    eventDescriptionCardClause(description, "message") ??
    eventDescriptionCardClause(description, "diff") ??
    textDescriptionCardClause(description, "message") ??
    textDescriptionCardClause(description, "diff");
  if (eventClause) return eventClause;

  if (description.startsWith("Reaction added")) {
    return `When a reaction is added${description.slice("Reaction added".length)}`;
  }
  if (description.endsWith(" reaction added")) {
    return `When someone reacts with ${description.slice(0, -" reaction added".length)}`;
  }
  if (
    description.startsWith("Any reaction ") &&
    description.endsWith(" added")
  ) {
    return `When ${description.slice(0, -" added".length).toLocaleLowerCase()} is added`;
  }
  return null;
}

function getActionCardClause(
  step: Record<string, unknown>,
  channelLabel?: string,
): string | null {
  const action = nonEmptyString(step.action);
  if (!action) return null;

  const actionLabel = isKnownActionType(action)
    ? actionTypeLabel(action)
    : null;
  const parsedStep: StepFormState | null = actionLabel
    ? {
        action: action as ActionType,
        id: nonEmptyString(step.id) ?? "step_1",
        duration: nonEmptyString(step.duration) ?? undefined,
        emoji: nonEmptyString(step.emoji) ?? undefined,
        from: nonEmptyString(step.from) ?? undefined,
        message: nonEmptyString(step.message) ?? undefined,
        method: nonEmptyString(step.method) ?? undefined,
        name: nonEmptyString(step.name) ?? undefined,
        text: nonEmptyString(step.text) ?? undefined,
        to: nonEmptyString(step.to) ?? undefined,
        topic: nonEmptyString(step.topic) ?? undefined,
        url: nonEmptyString(step.url) ?? undefined,
      }
    : null;
  const detail = parsedStep
    ? workflowStepDescription(parsedStep, {
        channelLabel,
        includeName: false,
      })
    : null;
  const configuredDetail =
    detail &&
    detail !== actionLabel &&
    detail !==
      (actionLabel
        ? `${actionLabel[0]}${actionLabel.slice(1).toLocaleLowerCase()}`
        : null)
      ? detail
      : null;

  switch (action) {
    case "delay": {
      return configuredDetail
        ? i18n.t("workflows.card.action-delay-detail", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-delay");
    }
    case "send_message":
      if (!configuredDetail)
        return i18n.t("workflows.card.action-send-message");
      return nonEmptyString(step.text)
        ? i18n.t("workflows.card.action-send-text", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-send-to-channel", {
            detail: configuredDetail,
          });
    case "call_webhook":
      return configuredDetail
        ? i18n.t("workflows.card.action-call-webhook-detail", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-call-webhook");
    case "send_dm":
      return configuredDetail
        ? i18n.t("workflows.card.action-send-dm-detail", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-send-dm");
    case "request_approval":
      return configuredDetail
        ? i18n.t("workflows.card.action-request-approval-detail", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-request-approval");
    case "add_reaction": {
      return configuredDetail
        ? i18n.t("workflows.card.action-add-reaction-detail", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-add-reaction");
    }
    case "set_channel_topic":
      return configuredDetail
        ? i18n.t("workflows.card.action-set-topic-detail", {
            detail: configuredDetail,
          })
        : i18n.t("workflows.card.action-set-topic");
    default: {
      return (actionLabel ?? humanizeIdentifier(action)).toLocaleLowerCase();
    }
  }
}

export function getWorkflowStepCount(
  definition: Record<string, unknown>,
): number {
  return getWorkflowSteps(definition).length;
}

/** Build a short plain-language label from the workflow's trigger and steps. */
export function getWorkflowCardLabel(
  definition: Record<string, unknown>,
  options: {
    actionChannelLabel?: string;
    triggerDescription?: string;
    triggerReaction?: string;
  } = {},
): string {
  const triggerConfig = getWorkflowTriggerConfig(definition);
  const triggerClause =
    getPresentedTriggerCardClause(options.triggerDescription) ??
    getPresentedTriggerCardClause(
      triggerConfig?.filter && !options.triggerReaction
        ? workflowTriggerDescription(triggerConfig, {
            omitUnresolvedReferences: true,
          })
        : undefined,
    ) ??
    getTriggerCardClause(definition, options.triggerReaction);
  const steps = getWorkflowSteps(definition);
  const firstAction = steps[0]
    ? getActionCardClause(steps[0], options.actionChannelLabel)
    : null;
  if (!firstAction) return triggerClause;

  const remainingStepCount = steps.length - 1;
  if (remainingStepCount === 0) {
    return i18n.t("workflows.card.label-with-action", {
      action: firstAction,
      trigger: triggerClause,
    });
  }

  return i18n.t("workflows.card.label-with-action-and-more", {
    action: firstAction,
    count: remainingStepCount,
    trigger: triggerClause,
  });
}

export function getWorkflowDescription(
  definition: Record<string, unknown>,
): string | null {
  return nonEmptyString(definition.description);
}

export function getWorkflowEnabled(
  definition: Record<string, unknown>,
): boolean {
  return definition.enabled !== false;
}

export function withWorkflowEnabled(
  definition: Record<string, unknown>,
  enabled: boolean,
): Record<string, unknown> {
  const updated = { ...definition };
  if (enabled) {
    delete updated.enabled;
  } else {
    updated.enabled = false;
  }
  return updated;
}

export function getWorkflowDisplayStatus(
  workflow: Workflow,
): Workflow["status"] | "disabled" {
  if (workflow.status !== "active") {
    return workflow.status;
  }

  return getWorkflowEnabled(workflow.definition) ? workflow.status : "disabled";
}

export function getWorkflowTriggerSummary(
  definition: Record<string, unknown>,
): string | null {
  const trigger = getWorkflowTriggerConfig(definition);
  if (trigger)
    return workflowTriggerDescription(trigger, {
      omitUnresolvedReferences: true,
    });

  const rawTrigger = asRecord(definition.trigger);
  const triggerType = nonEmptyString(rawTrigger?.on);
  if (!triggerType) return null;
  return TRIGGER_TYPES.includes(triggerType as TriggerType)
    ? triggerTypeLabel(triggerType as TriggerType)
    : triggerType;
}
