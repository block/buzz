/**
 * Live owner-scoped Assistant list state injected into each Bestie DM / popover
 * user turn so the agent can answer "what's on my todo list" from RHS storage
 * instead of session memory.
 */

import { presentBestieJobSchedule } from "./bestieDmRhsHelpers";
import { getBestieJobState } from "./bestieJobStore";
import type { BestieJob, BestieJobScope } from "./bestieJobTypes";
import { openReminders, openTodos } from "./bestieListStorage";
import {
  BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER,
  stripBestieOutboundHints,
} from "./bestieOutboundHints";
import { getBestieListState } from "./bestieListStore";
import type {
  BestieListItem,
  BestieListScope,
} from "./bestieListTypes";

export { BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER, stripBestieOutboundHints } from "./bestieOutboundHints";

/** Soft cap so the turn hint stays prompt-sized. */
export const BESTIE_LIVE_LIST_STATE_MAX_ITEMS = 30;

export type BestieLiveListStateSnapshot = {
  jobs: BestieJob[];
  openReminders: BestieListItem[];
  openTodos: BestieListItem[];
};

function formatDueAt(dueAt: number | null): string {
  if (dueAt == null) return "no due time";
  const date = new Date(dueAt * 1000);
  if (!Number.isFinite(date.getTime())) return "no due time";
  return date.toLocaleString();
}

function formatTodoLine(item: BestieListItem): string {
  const star = item.starred ? " ★" : "";
  const day = item.dayKey ? ` · day ${item.dayKey}` : "";
  return `- [${item.id}]${star} ${item.text}${day}`;
}

function formatReminderLine(item: BestieListItem): string {
  const repeat =
    item.repeat?.kind === "daily"
      ? "daily"
      : item.repeat?.kind === "weekly"
        ? `weekly/${item.repeat.weekday}`
        : "one-off";
  return `- [${item.id}] ${item.text} · due ${formatDueAt(item.dueAt)} · ${repeat}`;
}

function formatJobLine(job: BestieJob): string {
  const next =
    job.nextDueAt != null ? ` · next ${formatDueAt(job.nextDueAt)}` : "";
  const enabled = job.enabled ? "enabled" : "disabled";
  return `- [${job.id}] ${job.title} · ${presentBestieJobSchedule(job.schedule)}${next} · ${enabled}`;
}

function formatCappedLines(
  lines: string[],
  max: number,
): { body: string; omitted: number } {
  if (lines.length <= max) {
    return { body: lines.join("\n"), omitted: 0 };
  }
  const kept = lines.slice(0, max);
  return {
    body: kept.join("\n"),
    omitted: lines.length - max,
  };
}

/**
 * Pure formatter for tests — builds the turn-hint body from a snapshot.
 * Teach text tells the agent this block is authoritative live RHS state.
 */
export function formatBestieLiveListStateHint(
  snapshot: BestieLiveListStateSnapshot,
  options: { maxItems?: number } = {},
): string {
  const max = options.maxItems ?? BESTIE_LIVE_LIST_STATE_MAX_ITEMS;
  const todoLines = snapshot.openTodos.map(formatTodoLine);
  const reminderLines = snapshot.openReminders.map(formatReminderLine);
  const enabled = snapshot.jobs.filter((job) => job.enabled);
  const jobLines = enabled.map(formatJobLine);
  const todos = formatCappedLines(todoLines, max);
  const reminders = formatCappedLines(reminderLines, max);
  const jobs = formatCappedLines(jobLines, max);

  const todoBlock =
    todoLines.length === 0
      ? "(none)"
      : `${todos.body}${todos.omitted > 0 ? `\n- …and ${todos.omitted} more` : ""}`;
  const reminderBlock =
    reminderLines.length === 0
      ? "(none)"
      : `${reminders.body}${reminders.omitted > 0 ? `\n- …and ${reminders.omitted} more` : ""}`;
  const jobBlock =
    enabled.length === 0
      ? `(none enabled; ${snapshot.jobs.length} total)`
      : `${jobs.body}${jobs.omitted > 0 ? `\n- …and ${jobs.omitted} more` : ""}`;

  return `
${BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER}
Live Assistant list state from owner-scoped RHS storage (authoritative). When the user asks what is on their to-do list, reminders, or jobs, answer from THIS block. Do not claim you cannot query live state; do not prefer session chat memory over this snapshot.

Open to-dos (${todoLines.length}):
${todoBlock}

Open reminders (${reminderLines.length}):
${reminderBlock}

Jobs summary (${enabled.length} enabled / ${snapshot.jobs.length} total):
${jobBlock}
`;
}

/** Read current owner-scoped list + job state into a snapshot. */
export function readBestieLiveListStateSnapshot(
  scope: BestieListScope & BestieJobScope,
): BestieLiveListStateSnapshot {
  const list = getBestieListState(scope);
  const jobs = getBestieJobState(scope);
  return {
    jobs: jobs.jobs,
    openReminders: openReminders(list),
    openTodos: openTodos(list),
  };
}

/** Build the live-state turn hint for the given owner scope. */
export function bestieLiveListStateTurnHint(
  scope: BestieListScope & BestieJobScope,
  options: { maxItems?: number } = {},
): string {
  return formatBestieLiveListStateHint(
    readBestieLiveListStateSnapshot(scope),
    options,
  );
}

/** Append live list state when missing (every Bestie DM / popover user send). */
export function withBestieLiveListStateHint(
  content: string,
  scope: (BestieListScope & BestieJobScope) | null | undefined,
  options: { maxItems?: number } = {},
): string {
  if (!scope) return content;
  if (content.includes(BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER)) {
    return content;
  }
  return `${content.trimEnd()}${bestieLiveListStateTurnHint(scope, options)}`;
}

/** Alias for display strip of the live-state marker alone (tests). */
export function stripBestieLiveListStateHint(content: string): string {
  return stripBestieOutboundHints(content);
}
