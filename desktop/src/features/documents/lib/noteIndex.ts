/**
 * Resolving a wikilink target to a file in the vault.
 *
 * Obsidian's matching is looser than exact filename equality, and notes rely on
 * that: `[[meeting notes]]`, `[[Meeting-Notes]]` and `[[Meeting_Notes.md]]` all
 * point at `Meeting Notes.md`. Ported from Onyx's note index.
 */
import {
  baseName,
  joinPath,
  parentOf,
  relativeTo,
  stripMarkdownExtension,
} from "@/features/documents/lib/treeModel";

export type NoteIndex = {
  /** Normalized note name → every absolute path that matches it. */
  byName: ReadonlyMap<string, readonly string[]>;
  /** Vault-relative path (lowercased, no extension) → absolute path. */
  byRelativePath: ReadonlyMap<string, string>;
  vaultRoot: string;
};

export type ResolvedWikilink = {
  /** Absolute path, or the path the note *would* take if created. */
  path: string;
  /** Whether that file exists today. */
  exists: boolean;
};

/**
 * Obsidian-compatible name normalization: case-insensitive, extension-blind,
 * and treating `-`, `_` and space as the same character.
 */
export function normalizeName(name: string): string {
  return stripMarkdownExtension(name)
    .toLowerCase()
    .replace(/[-_]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function buildNoteIndex(
  vaultRoot: string,
  filePaths: readonly string[],
): NoteIndex {
  const byName = new Map<string, string[]>();
  const byRelativePath = new Map<string, string>();

  for (const path of filePaths) {
    const name = normalizeName(baseName(path));
    const existing = byName.get(name);
    if (existing) {
      existing.push(path);
    } else {
      byName.set(name, [path]);
    }

    const relative = stripMarkdownExtension(
      relativeTo(vaultRoot, path),
    ).toLowerCase();
    // First writer wins, so resolution is stable rather than dependent on
    // filesystem enumeration order.
    if (!byRelativePath.has(relative)) {
      byRelativePath.set(relative, path);
    }
  }

  return { byName, byRelativePath, vaultRoot };
}

/**
 * Resolves `target` against the index.
 *
 * Priority, matching Obsidian:
 *   1. A target containing a slash is a vault-relative path.
 *   2. Otherwise match by normalized name.
 *   3. On ties, prefer a note in the same folder as the linking file.
 *   4. Then prefer the shortest path — the one closest to the vault root.
 *
 * An unresolved target still returns a path: the file the link *would* create.
 * The caller decides whether to offer that, but the link needs somewhere to
 * point either way.
 */
export function resolveWikilink(
  target: string,
  fromPath: string,
  index: NoteIndex | null,
): ResolvedWikilink | null {
  const trimmed = target.trim();
  if (!index || !trimmed) return null;

  const withoutExtension = stripMarkdownExtension(trimmed);

  if (/[/\\]/.test(withoutExtension)) {
    const exact = index.byRelativePath.get(withoutExtension.toLowerCase());
    if (exact) return { exists: true, path: exact };
    return {
      exists: false,
      path: joinPath(index.vaultRoot, `${withoutExtension}.md`),
    };
  }

  const matches = index.byName.get(normalizeName(withoutExtension));
  if (!matches || matches.length === 0) {
    return {
      exists: false,
      path: joinPath(index.vaultRoot, `${withoutExtension}.md`),
    };
  }
  if (matches.length === 1) {
    return { exists: true, path: matches[0] };
  }

  const currentFolder = parentOf(fromPath);
  const sameFolder = matches.find((path) => parentOf(path) === currentFolder);
  if (sameFolder) return { exists: true, path: sameFolder };

  // Shortest path wins; ties break alphabetically so the result is stable
  // rather than dependent on insertion order.
  const sorted = [...matches].sort(
    (a, b) => a.length - b.length || a.localeCompare(b),
  );
  return { exists: true, path: sorted[0] };
}
