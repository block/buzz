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
 * Whether `line` mentions `name` as plain text, outside any wikilink.
 *
 * Word boundaries prevent "Note" matching inside "Notebook" — the noise that
 * makes an unlinked-mentions panel useless.
 *
 * `pattern` is supplied by the caller and reused across every line in the
 * vault, so its `lastIndex` must be reset here rather than relying on a fresh
 * object. `links` is likewise passed in: the caller has already parsed them to
 * test for linked mentions, and parsing each line twice was most of the cost of
 * a backlinks pass.
 */
function hasUnlinkedMention(
  line: string,
  pattern: RegExp,
  links: readonly { index: number; raw: string }[],
): boolean {
  pattern.lastIndex = 0;
  let match: RegExpExecArray | null = pattern.exec(line);
  while (match !== null) {
    const start = match.index;
    const insideLink = links.some(
      (link) => start >= link.index && start < link.index + link.raw.length,
    );
    if (!insideLink) return true;
    match = pattern.exec(line);
  }
  return false;
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
  // Compiled once for the whole vault rather than once per line. At ~75k lines
  // in a real vault, per-line compilation was the single largest cost here.
  const namePattern = new RegExp(`\\b${escapeRegExp(targetName)}\\b`, "gi");
  const lowerTarget = targetName.toLowerCase();

  for (const [sourcePath, text] of contents) {
    // A note is not its own backlink.
    if (sourcePath === targetPath) continue;

    // A note can only mention this one by linking to it or by naming it. Two
    // native substring scans rule out most of the vault before it is split into
    // lines at all, which is far cheaper than the per-line work below.
    const hasAnyLink = text.includes("[[");
    const lowerText = text.toLowerCase();
    if (!hasAnyLink && !lowerText.includes(lowerTarget)) continue;

    const sourceName = stripMarkdownExtension(baseName(sourcePath));
    const lines = text.split(/\r?\n/);

    for (const [offset, line] of lines.entries()) {
      // Same reasoning as above, one level down.
      const lineHasLink = line.includes("[[");
      if (!lineHasLink && !line.toLowerCase().includes(lowerTarget)) continue;

      const lineNumber = offset + 1;
      const links = lineHasLink ? parseWikilinks(line) : [];

      const resolvesHere = links.some((link) => {
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

      if (hasUnlinkedMention(line, namePattern, links)) {
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
