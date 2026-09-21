import {
  isDelete,
  isInsert,
  parseDiff,
  type DiffType,
  type FileData,
} from "react-diff-view";

import { i18n } from "@/i18n";

type ParsedDiffResult = {
  files: FileData[];
  parseError: boolean;
};

function isRenderableFile(file: FileData) {
  return file.hunks.length > 0 || Boolean(file.oldPath || file.newPath);
}

export function parseUnifiedDiff(content: string): ParsedDiffResult {
  if (!content.trim()) {
    return { files: [], parseError: false };
  }

  try {
    const files = parseDiff(content).filter(isRenderableFile);

    if (!files.length) {
      return { files: [], parseError: true };
    }

    return { files, parseError: false };
  } catch {
    return { files: [], parseError: true };
  }
}

/** Localized badge label for a diff change type (one literal key per type). */
export function diffTypeLabel(type: DiffType): string {
  switch (normalizeDiffType(type)) {
    case "add":
      return i18n.t("messages.diff.type-new-file");
    case "copy":
      return i18n.t("messages.diff.type-copied");
    case "delete":
      return i18n.t("messages.diff.type-deleted");
    case "rename":
      return i18n.t("messages.diff.type-renamed");
    default:
      return i18n.t("messages.diff.type-modified");
  }
}

export function getDiffFileLabel(
  file: FileData,
  fallbackFilePath?: string,
): string {
  const oldPath = file.oldPath === "/dev/null" ? undefined : file.oldPath;
  const newPath = file.newPath === "/dev/null" ? undefined : file.newPath;

  if (oldPath && newPath && oldPath !== newPath) {
    return `${oldPath} -> ${newPath}`;
  }

  return newPath || oldPath || fallbackFilePath || "diff";
}

export function shouldShowDiffFileHeader(
  label: string,
  fileCount: number,
  fallbackFilePath?: string,
): boolean {
  return fileCount > 1 || !fallbackFilePath || label !== fallbackFilePath;
}

/**
 * Change type behind the diff card's title badge. Set only when the diff is a
 * single file whose per-file header is collapsed (its label just repeats the
 * card title) and whose change type is notable — so "New file"/"Deleted" isn't
 * lost with the header. Callers resolve the text with `diffTypeLabel`.
 */
export function getDiffTitleBadgeType(
  content: string,
  fallbackFilePath?: string,
): DiffType | undefined {
  const { files } = parseUnifiedDiff(content);
  if (files.length !== 1) {
    return undefined;
  }

  const file = files[0];
  const label = getDiffFileLabel(file, fallbackFilePath);
  if (shouldShowDiffFileHeader(label, files.length, fallbackFilePath)) {
    return undefined;
  }

  const diffType = normalizeDiffType(file.type);
  return diffType === "modify" ? undefined : diffType;
}

export function countDiffFileChanges(file: FileData) {
  let additions = 0;
  let deletions = 0;

  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      if (isInsert(change)) {
        additions += 1;
      } else if (isDelete(change)) {
        deletions += 1;
      }
    }
  }

  return { additions, deletions };
}

export function normalizeDiffType(type: string | undefined): DiffType {
  switch (type) {
    case "add":
    case "copy":
    case "delete":
    case "rename":
      return type;
    default:
      return "modify";
  }
}
