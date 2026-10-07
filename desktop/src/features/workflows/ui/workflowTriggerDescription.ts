import { truncatePubkey } from "@/shared/lib/pubkey";
import { parseConditionExpressions } from "./workflowConditionExpression";
import { TRIGGER_LABELS } from "./workflowFormTypes";
import type { ParsedConditionExpression } from "./workflowConditionExpression";
import type { TriggerConfig } from "./workflowFormTypes";

const EVENT_PHRASES = {
  diff_posted: "Diff posted",
  message_posted: "Message posted",
  reaction_added: "Reaction added",
} as const;

export const TRIGGER_MESSAGE_LOADING_LABEL = "loading message";
export const TRIGGER_AUTHOR_LOADING_LABEL = "loading author";

function authorReference(
  condition: ParsedConditionExpression,
  authorLabel?: string,
  authorLoading?: boolean,
): string {
  if (authorLoading) return TRIGGER_AUTHOR_LOADING_LABEL;
  return authorLabel ?? truncatePubkey(condition.value);
}

function quotedValue(value: string): string {
  const normalized = value.trim().replaceAll(/\s+/g, " ");
  const abbreviated =
    normalized.length > 36 ? `${normalized.slice(0, 33)}...` : normalized;
  return `“${abbreviated}”`;
}

function messageReference(
  condition: ParsedConditionExpression,
  messageLabel?: string,
  messageLoading?: boolean,
): string {
  if (messageLoading) return TRIGGER_MESSAGE_LOADING_LABEL;
  return messageLabel
    ? quotedValue(messageLabel)
    : truncatePubkey(condition.value);
}

function textConditionDescription(
  eventPhrase: string,
  condition: ParsedConditionExpression,
): string {
  const subject = eventPhrase.replace(/ posted$/, "");
  const value = quotedValue(condition.value);
  switch (condition.operator) {
    case "contains":
      return `${subject} contains ${value}`;
    case "not_contains":
      return `${subject} doesn’t contain ${value}`;
    case "starts_with":
      return `${subject} starts with ${value}`;
    case "ends_with":
      return `${subject} ends with ${value}`;
    case "equals":
      return `${subject} ${value} is posted`;
    case "not_equals":
      return `${subject} with text other than ${value} posted`;
    case "is_not_empty":
      return `${subject} with text posted`;
    case "is_empty":
      return `${subject} without text posted`;
  }
}

/** Build the concise trigger summary rendered on the workflow canvas. */
export function workflowTriggerDescription(
  trigger: TriggerConfig,
  options: {
    authorLoading?: boolean;
    authorLabel?: string;
    messageLabel?: string;
    messageLoading?: boolean;
    omitUnresolvedReferences?: boolean;
  } = {},
): string {
  const baseLabel =
    EVENT_PHRASES[trigger.on as keyof typeof EVENT_PHRASES] ??
    TRIGGER_LABELS[trigger.on];
  const eventPhrase = EVENT_PHRASES[trigger.on as keyof typeof EVENT_PHRASES];
  if (!eventPhrase) return baseLabel;

  const conditions = trigger.filter
    ? parseConditionExpressions(trigger.filter, trigger.on)
    : [];
  if (!conditions || conditions.length === 0) {
    return trigger.on === "reaction_added" && trigger.emoji
      ? eventPhrase
      : baseLabel;
  }

  const condition = conditions[0];

  if (conditions.length > 1) {
    const authorCondition = conditions.find(
      ({ field }) => field === "trigger_author",
    );
    const textCondition = conditions.find(
      ({ field }) => field === "trigger_text",
    );
    const emojiCondition = conditions.find(
      ({ field }) => field === "trigger_emoji",
    );
    const messageCondition = conditions.find(
      ({ field }) => field === "trigger_message_id",
    );
    let description = textCondition
      ? textConditionDescription(eventPhrase, textCondition)
      : eventPhrase;

    if (emojiCondition) {
      description =
        emojiCondition.operator === "not_equals"
          ? `Any reaction except ${emojiCondition.value} added`
          : `${emojiCondition.value} reaction added`;
    }
    if (
      authorCondition &&
      (!options.omitUnresolvedReferences ||
        options.authorLabel ||
        options.authorLoading)
    ) {
      const author = authorReference(
        authorCondition,
        options.authorLabel,
        options.authorLoading,
      );
      const attribution =
        authorCondition.operator === "not_equals"
          ? ` by anyone except ${author}`
          : ` by ${author}`;
      const subject = eventPhrase.replace(/ posted$/, "");
      description =
        textCondition &&
        description.startsWith(subject) &&
        !/ (?:is )?posted$/.test(description)
          ? `${subject}${attribution}${description.slice(subject.length)}`
          : `${description}${attribution}`;
    }
    if (
      messageCondition &&
      (!options.omitUnresolvedReferences ||
        options.messageLabel ||
        options.messageLoading)
    ) {
      const message = messageReference(
        messageCondition,
        options.messageLabel,
        options.messageLoading,
      );
      description +=
        messageCondition.operator === "not_equals"
          ? ` anywhere except ${message}`
          : ` to ${message}`;
    }
    return description;
  }

  if (condition.field === "trigger_author") {
    if (
      options.omitUnresolvedReferences &&
      !options.authorLabel &&
      !options.authorLoading
    ) {
      return baseLabel;
    }
    const author = authorReference(
      condition,
      options.authorLabel,
      options.authorLoading,
    );
    return condition.operator === "not_equals"
      ? `${eventPhrase} by anyone except ${author}`
      : `${eventPhrase} by ${author}`;
  }

  if (condition.field === "trigger_text") {
    return textConditionDescription(eventPhrase, condition);
  }

  if (condition.field === "trigger_emoji") {
    return condition.operator === "not_equals"
      ? `Any reaction except ${condition.value} added`
      : `${condition.value} reaction added`;
  }

  if (condition.field === "trigger_message_id") {
    if (
      options.omitUnresolvedReferences &&
      !options.messageLabel &&
      !options.messageLoading
    ) {
      return baseLabel;
    }
    const message = messageReference(
      condition,
      options.messageLabel,
      options.messageLoading,
    );
    return condition.operator === "not_equals"
      ? `Reaction added anywhere except ${message}`
      : `Reaction added to ${message}`;
  }

  return baseLabel;
}
