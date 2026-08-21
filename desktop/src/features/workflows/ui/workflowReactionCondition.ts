import type { TriggerConfig } from "./workflowFormTypes";

/** Presents legacy reaction emoji constraints alongside structured filters. */
export function reactionConditionValue(trigger: TriggerConfig): string {
  const filter = trigger.filter?.trim() ?? "";
  const emoji = trigger.on === "reaction_added" ? trigger.emoji?.trim() : null;
  if (!emoji) return filter;
  const emojiCondition = `trigger_emoji == ${JSON.stringify(emoji)}`;
  return filter ? `${emojiCondition} && ${filter}` : emojiCondition;
}
