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

export type FileReadContentLine = {
  kind: "context" | "meta";
  text: string;
};

export type FileReadContent = {
  footerText: string;
  footerTitle: string;
  lines: FileReadContentLine[];
  path: string;
};

export function buildSkillReadContent(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): FileReadContent | null {
  if (descriptor.renderClass !== "skill-read") {
    return null;
  }

  const skillRef =
    getToolString(item.args, ["name"]) ??
    descriptor.preview ??
    descriptor.object;
  if (!skillRef) {
    return null;
  }

  const resultText = getResultText(item.result);
  if (!resultText.trim()) {
    return null;
  }

  const rawLines = trimTrailingEmptyLines(resultText.split(/\r?\n/));
  const lines =
    rawLines.length > 0
      ? rawLines.map((line) => ({
          kind: "context" as const,
          text: line,
        }))
      : [{ kind: "meta" as const, text: "No skill content returned." }];

  const footerText = skillRef.includes("/") ? skillRef : `${skillRef}/SKILL.md`;

  return {
    footerText,
    footerTitle: skillRef,
    lines,
    path: skillRef,
  };
}

export function buildFileReadContent(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): FileReadContent | null {
  if (descriptor.renderClass !== "file-read") {
    return null;
  }

  const path =
    getToolString(item.args, ["path", "file", "file_path", "target_file"]) ??
    descriptor.object ??
    descriptor.preview;
  if (!path) {
    return null;
  }

  const resultText = getResultText(item.result);
  if (!resultText.trim()) {
    return null;
  }

  const parsed = parseReadFileOutput(resultText, path);
  return {
    footerText: parsed.footerText,
    footerTitle: parsed.footerTitle,
    lines: parsed.lines,
    path,
  };
}

function parseReadFileOutput(resultText: string, path: string) {
  const rawLines = trimTrailingEmptyLines(resultText.split(/\r?\n/));
  const firstLine = rawLines[0] ?? "";
  const remainingLines = rawLines.slice(1);
  const hasRangeHeader =
    firstLine.startsWith(path) && /\s\(lines \d+-\d+ of \d+\)$/.test(firstLine);
  const contentLines = hasRangeHeader ? remainingLines : rawLines;
  const lines =
    contentLines.length > 0
      ? contentLines.map((line) => ({
          kind: isReadFileMetaLine(line)
            ? ("meta" as const)
            : ("context" as const),
          text: line,
        }))
      : [{ kind: "meta" as const, text: "No file content returned." }];

  return {
    footerText: hasRangeHeader ? firstLine : path,
    footerTitle: hasRangeHeader ? `${path}\n${firstLine}` : path,
    lines,
  };
}

function getResultText(result: string): string {
  const parsed = parseToolResultValue(result);
  if (typeof parsed === "string") {
    return parsed;
  }

  const record = asRecord(parsed);
  return (
    getRecordString(record, ["content", "text", "output", "stdout"]) ?? result
  );
}

function getRecordString(
  record: Record<string, unknown>,
  keys: string[],
): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") {
      return value;
    }
  }
  return null;
}

function isReadFileMetaLine(line: string) {
  return /^\[showing lines \d+-\d+ of \d+; use offset=\d+ to continue\]$/.test(
    line,
  );
}

function trimTrailingEmptyLines(lines: string[]) {
  let end = lines.length;
  while (end > 0 && lines[end - 1] === "") {
    end -= 1;
  }
  return end === lines.length ? lines : lines.slice(0, end);
}
