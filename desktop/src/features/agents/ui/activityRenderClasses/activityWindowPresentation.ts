import { currentCompanionWindowKind } from "@/app/companionWindow";

/** Dedicated feeds render navigation targets as readable, non-clickable content. */
export function activityWindowMarkdownInteractive(): boolean {
  return currentCompanionWindowKind() !== "agent-activity";
}
