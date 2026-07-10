import {
  isDelete,
  isInsert,
  parseDiff,
  type DiffType,
  type FileData,
} from "react-diff-view";

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

export const DIFF_TYPE_LABELS: Record<DiffType, string> = {
  add: "New file",
  copy: "Copied",
  delete: "Deleted",
  modify: "Modified",
  rename: "Renamed",
};

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
 * Badge for the diff card's title bar. Set only when the diff is a single
 * file whose per-file header is collapsed (its label just repeats the card
 * title) and whose change type is notable — so "New file"/"Deleted" isn't
 * lost with the header.
 */
export function getDiffTitleBadge(
  content: string,
  fallbackFilePath?: string,
): string | undefined {
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
  return diffType === "modify" ? undefined : DIFF_TYPE_LABELS[diffType];
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
