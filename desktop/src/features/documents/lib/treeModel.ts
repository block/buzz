/**
 * Pure helpers for the Documents file tree.
 *
 * Everything here is filesystem-free and framework-free so it can be unit
 * tested without Tauri.
 */
import type { VaultEntry } from "@/shared/api/vaultTypes";

/** One rendered row: a tree node plus where it sits. */
export type VaultTreeRow = {
  entry: VaultEntry;
  depth: number;
  isExpanded: boolean;
};

/** Strips a trailing `.md` / `.markdown`, for display and wikilink matching. */
export function stripMarkdownExtension(name: string): string {
  return name.replace(/\.(?:md|markdown)$/i, "");
}

/** The path separator used by `path`, defaulting to `/`. */
function separatorFor(path: string): string {
  return path.includes("\\") && !path.includes("/") ? "\\" : "/";
}

/** The final segment of a path. */
export function baseName(path: string): string {
  const normalized = path.replace(/[/\\]+$/, "");
  const index = Math.max(
    normalized.lastIndexOf("/"),
    normalized.lastIndexOf("\\"),
  );
  return index === -1 ? normalized : normalized.slice(index + 1);
}

/** The containing directory of a path, or `""` when there is none. */
export function parentOf(path: string): string {
  const normalized = path.replace(/[/\\]+$/, "");
  const index = Math.max(
    normalized.lastIndexOf("/"),
    normalized.lastIndexOf("\\"),
  );
  if (index <= 0) return index === 0 ? "/" : "";
  return normalized.slice(0, index);
}

/** Joins a directory and a child segment using the parent's separator. */
export function joinPath(directory: string, segment: string): string {
  const separator = separatorFor(directory);
  const trimmed = directory.replace(/[/\\]+$/, "");
  return `${trimmed}${separator}${segment}`;
}

/** `path` relative to `root`, or `path` unchanged when it sits outside. */
export function relativeTo(root: string, path: string): string {
  const trimmedRoot = root.replace(/[/\\]+$/, "");
  if (!path.startsWith(trimmedRoot)) return path;
  return path.slice(trimmedRoot.length).replace(/^[/\\]+/, "");
}

/**
 * Flattens the tree to the rows that are actually visible.
 *
 * Collapsed folders contribute a row but not their subtree, so a 10k-note vault
 * renders only what is on screen. Onyx rendered the whole tree recursively.
 */
export function flattenVisibleRows(
  entries: VaultEntry[],
  expandedPaths: ReadonlySet<string>,
  depth = 0,
): VaultTreeRow[] {
  const rows: VaultTreeRow[] = [];
  for (const entry of entries) {
    const isExpanded = entry.isDirectory && expandedPaths.has(entry.path);
    rows.push({ depth, entry, isExpanded });
    if (isExpanded && entry.children) {
      rows.push(
        ...flattenVisibleRows(entry.children, expandedPaths, depth + 1),
      );
    }
  }
  return rows;
}

/** Every folder path on the way from `root` down to `path`, exclusive of `path`. */
export function ancestorFolderPaths(root: string, path: string): string[] {
  const relative = relativeTo(root, path);
  if (!relative || relative === path) return [];
  const segments = relative.split(/[/\\]+/).filter(Boolean);
  segments.pop();

  const ancestors: string[] = [];
  let current = root.replace(/[/\\]+$/, "");
  for (const segment of segments) {
    current = joinPath(current, segment);
    ancestors.push(current);
  }
  return ancestors;
}

/** Depth-first walk yielding every file entry (directories excluded). */
export function collectFilePaths(entries: VaultEntry[]): string[] {
  const paths: string[] = [];
  const walk = (nodes: VaultEntry[]) => {
    for (const node of nodes) {
      if (node.isDirectory) {
        if (node.children) walk(node.children);
      } else {
        paths.push(node.path);
      }
    }
  };
  walk(entries);
  return paths;
}

/** Finds an entry by exact path. */
export function findEntry(
  entries: VaultEntry[],
  path: string,
): VaultEntry | null {
  for (const entry of entries) {
    if (entry.path === path) return entry;
    if (entry.children) {
      const found = findEntry(entry.children, path);
      if (found) return found;
    }
  }
  return null;
}

/**
 * Whether moving `sourcePath` to sit inside `destinationDir` is legal.
 *
 * Rejects a no-op (already there) and a move into the source's own subtree,
 * which would otherwise orphan the folder. The Rust side re-checks the second
 * case; this exists so the UI can refuse the drop rather than round-trip.
 */
export function canMoveInto(
  sourcePath: string,
  destinationDir: string,
): boolean {
  if (sourcePath === destinationDir) return false;
  if (parentOf(sourcePath) === destinationDir) return false;

  const sourcePrefix = `${sourcePath.replace(/[/\\]+$/, "")}/`;
  const normalizedDestination = `${destinationDir.replace(/[/\\]+$/, "")}/`;
  return !normalizedDestination.startsWith(sourcePrefix);
}
