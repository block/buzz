import { needsConjunctionGrouping } from "./workflowConditionExpression";
import type { TriggerConfig } from "./workflowFormTypes";

/** Presents legacy reaction emoji constraints alongside structured filters. */
export function reactionConditionValue(trigger: TriggerConfig): string {
  const filter = trigger.filter?.trim() ?? "";
  const emoji = trigger.on === "reaction_added" ? trigger.emoji?.trim() : null;
  if (!emoji) return filter;
  const emojiCondition = `trigger_emoji == ${JSON.stringify(emoji)}`;
  if (!filter) return emojiCondition;
  // The emoji constraint and the filter are ANDed at runtime, so a filter with
  // top-level alternatives must be grouped to preserve its original meaning.
  const guarded = needsConjunctionGrouping(filter) ? `(${filter})` : filter;
  return `${emojiCondition} && ${guarded}`;
}
