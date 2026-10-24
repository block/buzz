// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { i18n } from "@/i18n";
import { truncateNpub, truncatePubkey } from "@/shared/lib/pubkey";
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
  // The trigger author is a pubkey identity; the unresolved fallback renders
  // the npub compact, never raw hex.
  return authorLabel ?? truncateNpub(condition.value);
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
  // The referenced message is an event id, not a pubkey identity: it keeps
  // the generic hex truncation.
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
      return uiT("{0} contains {1}", { p0: subject, p1: value });
    case "not_contains":
      return uiT("{0} doesn’t contain {1}", { p0: subject, p1: value });
    case "starts_with":
      return uiT("{0} starts with {1}", { p0: subject, p1: value });
    case "ends_with":
      return uiT("{0} ends with {1}", { p0: subject, p1: value });
    case "equals":
      return uiT("{0} {1} is posted", { p0: subject, p1: value });
    case "not_equals":
      return uiT("{0} with text other than {1} posted", {
        p0: subject,
        p1: value,
      });
    case "is_not_empty":
      return uiT("{0} with text posted", { p0: subject });
    case "is_empty":
      return uiT("{0} without text posted", { p0: subject });
  }
}

export type WorkflowAuthorDescriptionSegments = {
  prefix: string;
  suffix: string;
};

/** Locate the generated author attribution without matching quoted user copy. */
export function splitWorkflowAuthorDescription(
  description: string,
  label: string,
): WorkflowAuthorDescriptionSegments | null {
  const markers = [
    { text: ` by anyone except ${label}`, offset: " by anyone except ".length },
    { text: ` by ${label}`, offset: " by ".length },
    { text: `${label} 이외의 사람이`, offset: 0 },
    { text: `${label}이(가)`, offset: 0 },
  ];

  for (const marker of markers) {
    let insideQuote = false;
    for (
      let index = 0;
      index <= description.length - marker.text.length;
      index += 1
    ) {
      const character = description[index];
      if (character === "“") insideQuote = true;
      if (character === "”") insideQuote = false;
      if (!insideQuote && description.startsWith(marker.text, index)) {
        const labelIndex = index + marker.offset;
        return {
          prefix: description.slice(0, labelIndex).trimEnd(),
          suffix: description.slice(labelIndex + label.length).trimStart(),
        };
      }
    }
  }

  return null;
}

/** Remove a reaction value from compact copy when the node icon already shows it. */
export function compactWorkflowTriggerDescription(
  description: string,
  triggerEmoji?: string,
): string {
  if (!triggerEmoji) return description;
  if (i18n.resolvedLanguage === "ko") {
    const prefix = `${triggerEmoji} 반응 추가됨`;
    return description.startsWith(prefix)
      ? `${uiT("Reaction added")}${description.slice(prefix.length)}`
      : description;
  }
  const emojiPrefix = `${triggerEmoji} reaction added`;
  return description.startsWith(emojiPrefix)
    ? uiT("Reaction added{0}", { p0: description.slice(emojiPrefix.length) })
    : description;
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
  if (i18n.resolvedLanguage === "ko") {
    const base = uiT(
      EVENT_PHRASES[trigger.on as keyof typeof EVENT_PHRASES] ??
        TRIGGER_LABELS[trigger.on],
    );
    const conditions = trigger.filter
      ? parseConditionExpressions(trigger.filter, trigger.on)
      : [];
    if (!conditions?.length) return base;
    // Build from parsed conditions, never by parsing translated prose.
    let description = base;
    for (const condition of conditions.filter(
      (item) => item.field === "trigger_text" || item.field === "trigger_emoji",
    )) {
      if (condition.field === "trigger_text") {
        description = textConditionDescription(
          trigger.on === "diff_posted" ? uiT("Diff") : uiT("Message"),
          condition,
        );
      } else {
        description =
          condition.operator === "not_equals"
            ? uiT("Any reaction except {0} added", { p0: condition.value })
            : uiT("{0} reaction added", { p0: condition.value });
      }
    }
    for (const condition of conditions.filter(
      (item) =>
        item.field === "trigger_author" || item.field === "trigger_message_id",
    )) {
      if (condition.field === "trigger_author") {
        if (
          options.omitUnresolvedReferences &&
          !options.authorLabel &&
          !options.authorLoading
        )
          continue;
        const author = options.authorLoading
          ? uiT("loading author")
          : authorReference(condition, options.authorLabel, false);
        description = uiT(
          condition.operator === "not_equals"
            ? "{0} by anyone except {1}"
            : "{0} by {1}",
          { p0: description, p1: author },
        );
      } else {
        if (
          options.omitUnresolvedReferences &&
          !options.messageLabel &&
          !options.messageLoading
        )
          continue;
        const message = options.messageLoading
          ? uiT("loading message")
          : messageReference(condition, options.messageLabel, false);
        description = uiT(
          condition.operator === "not_equals"
            ? "{0} anywhere except {1}"
            : "{0} to {1}",
          { p0: description, p1: message },
        );
      }
    }
    return description;
  }
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
      ? uiT("{0} by anyone except {1}", { p0: eventPhrase, p1: author })
      : uiT("{0} by {1}", { p0: eventPhrase, p1: author });
  }

  if (condition.field === "trigger_text") {
    return textConditionDescription(eventPhrase, condition);
  }

  if (condition.field === "trigger_emoji") {
    return condition.operator === "not_equals"
      ? uiT("Any reaction except {0} added", { p0: condition.value })
      : uiT("{0} reaction added", { p0: condition.value });
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
      ? uiT("Reaction added anywhere except {0}", { p0: message })
      : uiT("Reaction added to {0}", { p0: message });
  }

  return baseLabel;
}
