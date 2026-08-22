import type { TranscriptItem } from "./agentSessionTypes";
import { getToolString } from "./agentSessionUtils";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

/**
 * Shell entry points that are almost always scaffolding for the real work
 * (inspecting the tree, reading a file, checking a binary exists) rather than
 * the work itself. Matched against the FIRST token of the command only, so
 * `grep -rn …` is internal but a project script named `grep-report` is not.
 */
const INTERNAL_SHELL_COMMANDS = new Set([
  "awk",
  "basename",
  "cat",
  "cd",
  "chmod",
  "cp",
  "dirname",
  "echo",
  "env",
  "file",
  "find",
  "grep",
  "head",
  "ls",
  "mkdir",
  "mv",
  "printf",
  "pwd",
  "rg",
  "sed",
  "sort",
  "stat",
  "tail",
  "tr",
  "uniq",
  "wc",
  "which",
]);

/** Compound shells are pipelines/chains — plumbing, not a narrative step. */
const COMPOUND_SHELL_MARKERS = ["&&", "||", "2>&1", " | ", ";"];

/**
 * A group must be at least this long before any child is hidden. Below it, the
 * group is already short enough to read, and hiding would trade one row of
 * signal for one row of chrome.
 */
export const INTERNAL_STEP_GROUP_MINIMUM = 4;

/** At least this many children must be hideable for the toggle to earn a row. */
export const INTERNAL_STEP_MINIMUM_HIDDEN = 2;

function isCompletedAndClean(item: TranscriptItem): boolean {
  if (item.type !== "tool") return false;
  if (item.isError || item.status === "failed") return false;
  return item.status === "completed";
}

/**
 * Whether a single completed step is low-signal enough to fold away.
 *
 * Deliberately conservative: only successful, finished, shell-flavored
 * scaffolding qualifies. A failure, an in-flight step, a file edit, a relay op,
 * or anything the classifier could not identify always stays visible — the cost
 * of hiding something consequential is far higher than the cost of one extra
 * routine row.
 */
export function isLowSignalToolStep(item: TranscriptItem): boolean {
  if (!isCompletedAndClean(item)) return false;
  const tool = item as ToolItem;
  const renderClass = tool.renderClass;
  if (renderClass !== "shell") return false;

  const command = getToolString(tool.args, ["command"])?.trim();
  if (!command) return false;

  if (COMPOUND_SHELL_MARKERS.some((marker) => command.includes(marker))) {
    return true;
  }

  const firstToken = command.split(/\s+/)[0]?.toLowerCase() ?? "";
  const bareToken = firstToken.slice(firstToken.lastIndexOf("/") + 1);
  return INTERNAL_SHELL_COMMANDS.has(bareToken);
}

export type ToolRunPartition = {
  /** Steps always rendered when the group is open. */
  primaryItems: TranscriptItem[];
  /** Low-signal steps folded behind progressive disclosure. */
  hiddenItems: TranscriptItem[];
};

/**
 * Split a group's children into always-visible and foldable steps.
 *
 * Returns everything as primary (an empty `hiddenItems`) unless every gate
 * passes: the group is long enough, at least two children would hide, and at
 * least one primary step remains. A group that folds down to nothing but a
 * disclosure toggle is worse than the group it replaced.
 *
 * `expandedItemIds` pins steps the reader opened, so disclosure never yanks
 * away something being read.
 */
export function partitionToolRunSteps(
  items: readonly TranscriptItem[],
  expandedItemIds: readonly string[] = [],
): ToolRunPartition {
  const all = [...items];
  if (all.length < INTERNAL_STEP_GROUP_MINIMUM) {
    return { primaryItems: all, hiddenItems: [] };
  }

  const pinned = new Set(expandedItemIds);
  const primaryItems: TranscriptItem[] = [];
  const hiddenItems: TranscriptItem[] = [];

  for (const item of all) {
    if (!pinned.has(item.id) && isLowSignalToolStep(item)) {
      hiddenItems.push(item);
      continue;
    }
    primaryItems.push(item);
  }

  if (
    primaryItems.length === 0 ||
    hiddenItems.length < INTERNAL_STEP_MINIMUM_HIDDEN
  ) {
    return { primaryItems: all, hiddenItems: [] };
  }

  return { primaryItems, hiddenItems };
}

export type ToolRunArtifact = {
  /** Path as the agent referenced it — used for the tooltip. */
  path: string;
  /** Trailing filename segment — the chip label. */
  filename: string;
  /** Id of the step that produced or touched it, so a chip can reveal it. */
  itemId: string;
  kind: "edit" | "read";
};

function artifactPathForItem(item: TranscriptItem): {
  path: string;
  kind: "edit" | "read";
} | null {
  if (item.type !== "tool" || item.isError) return null;
  const renderClass = item.renderClass;
  if (renderClass !== "file-edit" && renderClass !== "file-read") return null;

  const path =
    getToolString(item.args, ["path", "file_path", "filePath"]) ??
    item.descriptor.object ??
    null;
  if (!path) return null;

  return { path, kind: renderClass === "file-edit" ? "edit" : "read" };
}

function filenameForPath(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const segment = trimmed.slice(trimmed.lastIndexOf("/") + 1);
  return segment || trimmed;
}

/**
 * Every file a group touched, one entry per distinct path.
 *
 * These render OUTSIDE the group's collapsible body: a file the agent worked on
 * is a durable outcome, not a detail of how the group happened to be batched,
 * so it must not disappear when the group collapses and must look the same
 * whether the group holds one step or twenty. An edit wins over a read for the
 * same path — writing is the more consequential fact about that file.
 */
export function collectToolRunArtifacts(
  items: readonly TranscriptItem[],
): ToolRunArtifact[] {
  const byPath = new Map<string, ToolRunArtifact>();

  for (const item of items) {
    const resolved = artifactPathForItem(item);
    if (!resolved) continue;

    const existing = byPath.get(resolved.path);
    if (existing && !(existing.kind === "read" && resolved.kind === "edit")) {
      continue;
    }

    byPath.set(resolved.path, {
      path: resolved.path,
      filename: filenameForPath(resolved.path),
      itemId: item.id,
      kind: resolved.kind,
    });
  }

  return [...byPath.values()];
}
