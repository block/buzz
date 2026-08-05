/**
 * Backlinks: which notes point at this one, and which merely name it.
 *
 * Two kinds, following Obsidian:
 *
 *  - **Linked mentions** — an actual `[[wikilink]]` that resolves here.
 *  - **Unlinked mentions** — the note's name appearing as plain text, which is
 *    usually a link the author forgot to make.
 */
import {
  baseName,
  stripMarkdownExtension,
} from "@/features/documents/lib/treeModel";
import {
  normalizeName,
  resolveWikilink,
  type NoteIndex,
} from "@/features/documents/lib/noteIndex";
import { parseWikilinks } from "@/features/documents/lib/wikilinkSyntax";

export type MentionKind = "linked" | "unlinked";

export type Mention = {
  /** Absolute path of the note containing the mention. */
  sourcePath: string;
  /** Display name of that note. */
  sourceName: string;
  kind: MentionKind;
  /** Line the mention sits on, for preview. */
  line: string;
  /** 1-based line number. */
  lineNumber: number;
};

export type Backlinks = {
  linked: Mention[];
  unlinked: Mention[];
};

/** Escapes a string for literal use inside a regex. */
function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Finds plain-text occurrences of `name`, ignoring any inside a wikilink.
 *
 * Word boundaries prevent "Note" matching inside "Notebook" — the noise that
 * makes an unlinked-mentions panel useless.
 */
function findUnlinkedRanges(line: string, name: string): number[] {
  const pattern = new RegExp(`\\b${escapeRegExp(name)}\\b`, "gi");
  const linkRanges = parseWikilinks(line).map((link) => [
    link.index,
    link.index + link.raw.length,
  ]);

  const hits: number[] = [];
  let match: RegExpExecArray | null = pattern.exec(line);
  while (match !== null) {
    const start = match.index;
    const insideLink = linkRanges.some(
      ([from, to]) => start >= from && start < to,
    );
    if (!insideLink) hits.push(start);
    match = pattern.exec(line);
  }
  return hits;
}

/**
 * Collects mentions of `targetPath` across the vault.
 *
 * `contents` maps absolute path → raw note text.
 */
export function getBacklinks({
  contents,
  index,
  targetPath,
}: {
  contents: ReadonlyMap<string, string>;
  index: NoteIndex | null;
  targetPath: string;
}): Backlinks {
  const linked: Mention[] = [];
  const unlinked: Mention[] = [];

  const targetName = stripMarkdownExtension(baseName(targetPath));
  const normalizedTarget = normalizeName(targetName);

  for (const [sourcePath, text] of contents) {
    // A note is not its own backlink.
    if (sourcePath === targetPath) continue;

    const sourceName = stripMarkdownExtension(baseName(sourcePath));
    const lines = text.split(/\r?\n/);

    for (const [offset, line] of lines.entries()) {
      const lineNumber = offset + 1;

      const resolvesHere = parseWikilinks(line).some((link) => {
        if (!link.target) return false;
        const resolved = resolveWikilink(link.target, sourcePath, index);
        // Fall back to name comparison when there is no index, so backlinks
        // still work before the corpus finishes loading.
        return resolved
          ? resolved.path === targetPath
          : normalizeName(link.target) === normalizedTarget;
      });

      if (resolvesHere) {
        linked.push({
          kind: "linked",
          line,
          lineNumber,
          sourceName,
          sourcePath,
        });
        continue;
      }

      if (findUnlinkedRanges(line, targetName).length > 0) {
        unlinked.push({
          kind: "unlinked",
          line,
          lineNumber,
          sourceName,
          sourcePath,
        });
      }
    }
  }

  return { linked, unlinked };
}

/** Groups mentions by their source note, preserving first-seen order. */
export function groupMentionsBySource(
  mentions: readonly Mention[],
): Array<{ sourceName: string; sourcePath: string; mentions: Mention[] }> {
  const groups = new Map<
    string,
    { sourceName: string; sourcePath: string; mentions: Mention[] }
  >();

  for (const mention of mentions) {
    const group = groups.get(mention.sourcePath);
    if (group) {
      group.mentions.push(mention);
    } else {
      groups.set(mention.sourcePath, {
        mentions: [mention],
        sourceName: mention.sourceName,
        sourcePath: mention.sourcePath,
      });
    }
  }

  return [...groups.values()];
}
