/**
 * Parse the markdown checklist a `plan` transcript item carries into structured
 * entries, so the `conversation` variant can render it as a checklist card
 * instead of a block of markdown.
 *
 * The text is produced by `formatPlanEntry` in agentSessionTranscriptHelpers:
 * `- [x] content` for completed entries, `- [ ] content` otherwise, with an
 * ` (in progress)` suffix when the ACP entry status is `in_progress`. Adapters
 * that send free-form `content` text instead of `entries[]` yield text with no
 * checklist lines at all — those return no entries and the card falls back to
 * rendering the raw markdown.
 */
export type PlanChecklistEntry = {
  label: string;
  status: "completed" | "in_progress" | "pending";
};

export type PlanChecklist = {
  completedCount: number;
  entries: PlanChecklistEntry[];
};

const CHECKLIST_LINE = /^\s*[-*]\s+\[([ xX])\]\s*(.*)$/;
const IN_PROGRESS_SUFFIX = /\s*\(in progress\)\s*$/i;

export function parsePlanChecklist(text: string): PlanChecklist {
  const entries: PlanChecklistEntry[] = [];

  for (const line of text.split(/\r?\n/)) {
    const match = line.match(CHECKLIST_LINE);
    if (!match) continue;
    const completed = match[1].toLowerCase() === "x";
    const rawLabel = match[2].trim();
    const inProgress = !completed && IN_PROGRESS_SUFFIX.test(rawLabel);
    entries.push({
      label: rawLabel.replace(IN_PROGRESS_SUFFIX, "").trim(),
      status: completed ? "completed" : inProgress ? "in_progress" : "pending",
    });
  }

  return {
    completedCount: entries.filter((entry) => entry.status === "completed")
      .length,
    entries,
  };
}

/** "2/5 complete" progress caption, or null when there is nothing to count. */
export function formatPlanChecklistProgress(
  checklist: PlanChecklist,
): string | null {
  if (checklist.entries.length === 0) return null;
  return `${checklist.completedCount}/${checklist.entries.length} complete`;
}
