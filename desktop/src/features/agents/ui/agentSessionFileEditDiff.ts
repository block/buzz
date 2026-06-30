import type {
  AgentActivityDescriptor,
  TranscriptItem,
} from "./agentSessionTypes";
import {
  asRecord,
  getToolString,
  parseToolResultValue,
} from "./agentSessionUtils";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

export type FileEditDiffLineKind = "add" | "remove" | "context" | "meta";

export type FileEditDiffLine = {
  kind: FileEditDiffLineKind;
  text: string;
};

export type FileEditDiffSummary = {
  path: string;
  filename: string;
  additions: number;
  deletions: number;
};

export type FileEditDiff = FileEditDiffSummary & {
  lines: FileEditDiffLine[];
};

const SHIKI_ADD_RE = /\s*\/\/\s*\[!code\s*\+\+\]\s*$/;
const SHIKI_REMOVE_RE = /\s*\/\/\s*\[!code\s*--\]\s*$/;

export function buildFileEditDiff(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): FileEditDiff | null {
  if (descriptor.renderClass !== "file-edit") {
    return null;
  }

  const resultText = getResultText(item.result);
  const path =
    getToolString(item.args, ["path", "file", "file_path", "target_file"]) ??
    descriptor.object ??
    descriptor.preview ??
    getDiffPath(resultText);

  if (!path) {
    return null;
  }

  const lines = getDiffLines(resultText);
  const stats = getDiffStats(resultText, lines);
  if (!stats) {
    return null;
  }

  return {
    path,
    filename: basename(path),
    additions: stats.additions,
    deletions: stats.deletions,
    lines,
  };
}

function getResultText(result: string): string {
  const parsed = parseToolResultValue(result);
  if (typeof parsed === "string") {
    return parsed;
  }

  const record = asRecord(parsed);
  const output = [
    getToolString(record, ["stdout", "output", "text"]),
    getToolString(record, ["stderr"]),
  ]
    .filter((value): value is string => value != null)
    .join("\n");

  return output || result;
}

function getDiffPath(text: string): string | null {
  for (const line of text.split(/\r?\n/)) {
    const match = line.match(/^\+\+\+\s+(?:b\/)?(.+)$/);
    if (match?.[1] && match[1] !== "/dev/null") {
      return match[1].trim();
    }
  }

  for (const line of text.split(/\r?\n/)) {
    const match = line.match(/^---\s+(?:a\/)?(.+)$/);
    if (match?.[1] && match[1] !== "/dev/null") {
      return match[1].trim();
    }
  }

  return null;
}

function getDiffLines(text: string): FileEditDiffLine[] {
  const rawLines = text.split(/\r?\n/);
  const hasUnifiedDiff = rawLines.some((line) =>
    /^(diff --git|--- |\+\+\+ |@@)/.test(line),
  );
  const lines: FileEditDiffLine[] = [];
  let inUnifiedDiff = false;

  for (const rawLine of rawLines) {
    const shikiKind = getShikiDiffKind(rawLine);
    if (shikiKind) {
      lines.push({
        kind: shikiKind,
        text: rawLine.replace(
          shikiKind === "add" ? SHIKI_ADD_RE : SHIKI_REMOVE_RE,
          "",
        ),
      });
      continue;
    }

    if (hasUnifiedDiff && !inUnifiedDiff) {
      if (!/^(diff --git|--- |\+\+\+ |@@)/.test(rawLine)) {
        continue;
      }
      inUnifiedDiff = true;
    }

    if (hasUnifiedDiff) {
      lines.push(classifyUnifiedDiffLine(rawLine));
      continue;
    }

    lines.push({ kind: "context", text: rawLine });
  }

  return trimTrailingEmptyContextLines(
    lines.filter((line) => line.text.length > 0 || line.kind !== "meta"),
  );
}

function trimTrailingEmptyContextLines(
  lines: FileEditDiffLine[],
): FileEditDiffLine[] {
  let end = lines.length;
  while (end > 0) {
    const line = lines[end - 1];
    if (line.kind !== "context" || line.text.length > 0) {
      break;
    }
    end -= 1;
  }
  return end === lines.length ? lines : lines.slice(0, end);
}

function getShikiDiffKind(line: string): "add" | "remove" | null {
  if (SHIKI_ADD_RE.test(line)) return "add";
  if (SHIKI_REMOVE_RE.test(line)) return "remove";
  return null;
}

function classifyUnifiedDiffLine(line: string): FileEditDiffLine {
  if (/^(diff --git|--- |\+\+\+ |@@)/.test(line)) {
    return { kind: "meta", text: line };
  }
  if (line.startsWith("+")) {
    return { kind: "add", text: line };
  }
  if (line.startsWith("-")) {
    return { kind: "remove", text: line };
  }
  return { kind: "context", text: line };
}

function getDiffStats(
  text: string,
  lines: FileEditDiffLine[],
): Pick<FileEditDiffSummary, "additions" | "deletions"> | null {
  const additions = lines.filter((line) => line.kind === "add").length;
  const deletions = lines.filter((line) => line.kind === "remove").length;

  if (additions > 0 || deletions > 0) {
    return { additions, deletions };
  }

  const statAdditions = text.match(/(\d+)\s+insertions?\(\+\)/);
  const statDeletions = text.match(/(\d+)\s+deletions?\(-\)/);
  if (statAdditions || statDeletions) {
    return {
      additions: statAdditions ? Number(statAdditions[1]) : 0,
      deletions: statDeletions ? Number(statDeletions[1]) : 0,
    };
  }

  return null;
}

function basename(path: string) {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}
