import { Extension } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { Plugin, PluginKey, type Transaction } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

export const mentionHighlightKey = new PluginKey("mentionHighlight");

export type MentionDeleteRange = { from: number; to: number };

type MentionDeleteDecorationSpec = {
  mentionDelete?: {
    fromOffset: number;
    toOffset: number;
  };
};

/**
 * TipTap extension that applies inline `mention-highlight` decorations
 * to `@Name` and `#channel-name` patterns in the document.
 *
 * Accepts `names` (display names) and `channelNames` storage options.
 * On every doc update the plugin scans text nodes and decorates matches.
 */
export const MentionHighlightExtension = Extension.create({
  name: "mentionHighlight",

  addStorage() {
    return {
      names: [] as string[],
      agentNames: [] as string[],
      channelNames: [] as string[],
    };
  },

  addProseMirrorPlugins() {
    const extension = this;

    return [
      new Plugin({
        key: mentionHighlightKey,
        state: {
          init(_, state) {
            return buildDecorations(
              state.doc,
              extension.storage.names,
              extension.storage.agentNames,
              extension.storage.channelNames,
            );
          },
          apply(tr, oldDecorations) {
            // Names/channels changed — full rebuild required.
            if (tr.getMeta(mentionHighlightKey)) {
              return buildDecorations(
                tr.doc,
                extension.storage.names,
                extension.storage.agentNames,
                extension.storage.channelNames,
              );
            }

            if (!tr.docChanged) {
              return oldDecorations;
            }

            // Check if the edit touches a mention boundary. If the changed
            // ranges contain `@` or `#` (either before or after the edit),
            // a mention may have been created, modified, or destroyed — do
            // a full rebuild. Otherwise, just map existing decoration
            // positions through the transaction mapping (cheap, no DOM churn).
            if (editAffectsMentionBoundary(tr)) {
              return buildDecorations(
                tr.doc,
                extension.storage.names,
                extension.storage.agentNames,
                extension.storage.channelNames,
              );
            }

            // If an edit intersects an existing decoration, the mapped
            // decoration may become stale (e.g. @Max → @Marx). Rebuild.
            if (editIntersectsDecoration(tr, oldDecorations)) {
              return buildDecorations(
                tr.doc,
                extension.storage.names,
                extension.storage.agentNames,
                extension.storage.channelNames,
              );
            }

            return oldDecorations.map(tr.mapping, tr.doc);
          },
        },
        props: {
          decorations(state) {
            return this.getState(state) ?? DecorationSet.empty;
          },
        },
      }),
    ];
  },
});

/**
 * Build highlight patterns for @Name and #channel-name matching.
 * Exported for testing — the patterns are the core logic of this extension.
 */
export function buildHighlightPatterns(
  names: string[],
  channelNames: string[],
): RegExp[] {
  const patterns: RegExp[] = [];

  if (names.length > 0) {
    const sortedNames = [...names].sort((a, b) => b.length - a.length);
    const escapedNames = sortedNames.map((n) =>
      n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
    );
    patterns.push(
      new RegExp(`(?:^|(?<=\\s))@(${escapedNames.join("|")})(?=\\W|$)`, "gi"),
    );
  }

  if (channelNames.length > 0) {
    const sortedChannels = [...channelNames].sort(
      (a, b) => b.length - a.length,
    );
    const escapedChannels = sortedChannels.map((n) =>
      n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
    );
    patterns.push(
      new RegExp(
        `(?:^|(?<=\\s))#(${escapedChannels.join("|")})(?=\\W|$)`,
        "gi",
      ),
    );
  }

  return patterns;
}

/**
 * Find all highlight matches in a text string given a set of patterns.
 * Returns an array of { from, to } offsets relative to the text start.
 * Exported for testing.
 */
export function findHighlightMatches(
  text: string,
  patterns: RegExp[],
): { from: number; to: number; match: string }[] {
  const results: { from: number; to: number; match: string }[] = [];
  for (const pattern of patterns) {
    pattern.lastIndex = 0;
    let m: RegExpExecArray | null = pattern.exec(text);
    while (m !== null) {
      results.push({ from: m.index, to: m.index + m[0].length, match: m[0] });
      m = pattern.exec(text);
    }
  }
  return results;
}

/**
 * Returns the full @mention range touched by Backspace at `cursor`, if any.
 *
 * The stored offsets are relative to each decoration's current mapped
 * position, so they stay valid when ProseMirror maps decorations after edits
 * elsewhere in the document.
 */
export function findMentionDeleteRangeBeforeCursor(
  decorations: DecorationSet,
  cursor: number,
): MentionDeleteRange | null {
  if (cursor <= 0) return null;

  const touchedDecorations = decorations.find(
    cursor - 1,
    cursor,
    hasMentionDeleteSpec,
  );

  for (const decoration of touchedDecorations) {
    const range = getMentionDeleteRange(decoration);
    if (range && range.from < cursor && cursor <= range.to) {
      return range;
    }
  }

  return null;
}

export function findMentionBackspaceDeleteRange(
  doc: ProseMirrorNode,
  decorations: DecorationSet,
  cursor: number,
): MentionDeleteRange | null {
  const directRange = findMentionDeleteRangeBeforeCursor(decorations, cursor);
  if (directRange) return directRange;

  if (cursor <= 1) return null;

  // Autocomplete inserts a separator space after @mentions. When the caret is
  // in that natural post-insert position, make one Backspace remove the tag
  // and its separator instead of requiring a first press just to eat the space.
  const previousChar = doc.textBetween(cursor - 1, cursor, "\n", "\0");
  if (previousChar !== " ") return null;

  const beforeSpaceRange = findMentionDeleteRangeBeforeCursor(
    decorations,
    cursor - 1,
  );
  if (!beforeSpaceRange || beforeSpaceRange.to !== cursor - 1) {
    return null;
  }

  return { from: beforeSpaceRange.from, to: cursor };
}

/**
 * Returns true if the transaction's changed ranges touch text that contains
 * `@` or `#` — meaning a mention/channel-link boundary may have been
 * created, modified, or destroyed and we need a full decoration rebuild.
 *
 * We check both the old content (in case a mention was deleted/split) and
 * the new content (in case one was just typed). Uses a simple approach:
 * iterate each step's changed ranges via the first stepMap (sufficient for
 * the single-step transactions a chat composer produces on each keystroke).
 */
function editAffectsMentionBoundary(tr: Transaction): boolean {
  const mentionChars = /[@#]/;

  // For each step, check old and new text in the changed range.
  // stepMap.forEach gives (oldFrom, oldTo, newFrom, newTo) where old
  // positions are in the doc before that step and new positions are in
  // the doc after that step.
  for (let i = 0; i < tr.steps.length; i++) {
    const map = tr.mapping.maps[i];

    let found = false;
    map.forEach((oldFrom, oldTo, newFrom, newTo) => {
      if (found) return;

      // Check new doc text in the affected range
      const clampedNewTo = Math.min(newTo, tr.doc.content.size);
      const clampedNewFrom = Math.min(newFrom, clampedNewTo);
      if (clampedNewFrom < clampedNewTo) {
        const newText = tr.doc.textBetween(
          clampedNewFrom,
          clampedNewTo,
          "\n",
          "\0",
        );
        if (mentionChars.test(newText)) {
          found = true;
          return;
        }
      }

      // Check old doc text in the affected range
      const clampedOldTo = Math.min(oldTo, tr.before.content.size);
      const clampedOldFrom = Math.min(oldFrom, clampedOldTo);
      if (clampedOldFrom < clampedOldTo) {
        const oldText = tr.before.textBetween(
          clampedOldFrom,
          clampedOldTo,
          "\n",
          "\0",
        );
        if (mentionChars.test(oldText)) {
          found = true;
        }
      }
    });

    if (found) return true;
  }

  return false;
}

/**
 * Returns true if any changed range in the transaction overlaps an existing
 * mention decoration. In that case the mapped decoration would be stale
 * (e.g. @Max edited to @Marx) and we need a full rebuild.
 */
function editIntersectsDecoration(
  tr: Transaction,
  decorations: DecorationSet,
): boolean {
  let hit = false;
  tr.mapping.maps.forEach((map) => {
    map.forEach((oldFrom, oldTo) => {
      if (hit) return;
      if (decorations.find(oldFrom, oldTo).length > 0) {
        hit = true;
      }
    });
  });
  return hit;
}

function buildDecorations(
  doc: Parameters<typeof DecorationSet.create>[0],
  names: string[],
  agentNames: string[],
  channelNames: string[],
): DecorationSet {
  if (
    names.length === 0 &&
    agentNames.length === 0 &&
    channelNames.length === 0
  )
    return DecorationSet.empty;

  const decorations: Decoration[] = [];
  const agentNameSet = new Set(
    agentNames.map((name) => name.trim().toLowerCase()).filter(Boolean),
  );
  const nonAgentNames = names.filter(
    (name) => !agentNameSet.has(name.trim().toLowerCase()),
  );
  const mentionPatterns = buildHighlightPatterns(nonAgentNames, []);
  const agentMentionPatterns = buildHighlightPatterns(agentNames, []);
  const channelPatterns = buildHighlightPatterns([], channelNames);

  doc.descendants((node, pos) => {
    if (!node.isText || !node.text) return;

    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      mentionPatterns,
      "mention-highlight",
      { deleteAsMention: true },
    );
    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      agentMentionPatterns,
      "mention-highlight agent-mention-highlight",
      { deleteAsMention: true, hideMentionPrefix: true },
    );
    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      channelPatterns,
      "mention-highlight",
    );
  });

  return DecorationSet.create(doc, decorations);
}

function addMatchesForPatterns(
  decorations: Decoration[],
  text: string,
  position: number,
  patterns: RegExp[],
  className: string,
  options?: { deleteAsMention?: boolean; hideMentionPrefix?: boolean },
) {
  for (const pattern of patterns) {
    pattern.lastIndex = 0;
    let match: RegExpExecArray | null = pattern.exec(text);
    while (match !== null) {
      const from = position + match.index;
      const to = from + match[0].length;
      const deleteRangeLength = to - from;
      if (options?.hideMentionPrefix && match[0].startsWith("@")) {
        decorations.push(
          Decoration.inline(
            from,
            from + 1,
            {
              class: "agent-mention-at-hidden",
            },
            options.deleteAsMention
              ? mentionDeleteSpec(0, deleteRangeLength - 1)
              : undefined,
          ),
        );
        decorations.push(
          Decoration.inline(
            from + 1,
            to,
            { class: className },
            options.deleteAsMention ? mentionDeleteSpec(-1, 0) : undefined,
          ),
        );
      } else {
        decorations.push(
          Decoration.inline(
            from,
            to,
            { class: className },
            options?.deleteAsMention ? mentionDeleteSpec(0, 0) : undefined,
          ),
        );
      }
      match = pattern.exec(text);
    }
  }
}

function mentionDeleteSpec(
  fromOffset: number,
  toOffset: number,
): MentionDeleteDecorationSpec {
  return {
    mentionDelete: {
      fromOffset,
      toOffset,
    },
  };
}

function hasMentionDeleteSpec(spec: unknown): boolean {
  return getMentionDeleteOffsets(spec) !== null;
}

function getMentionDeleteRange(
  decoration: Decoration,
): MentionDeleteRange | null {
  const offsets = getMentionDeleteOffsets(decoration.spec);
  if (!offsets) return null;

  const from = decoration.from + offsets.fromOffset;
  const to = decoration.to + offsets.toOffset;
  if (!Number.isInteger(from) || !Number.isInteger(to) || from >= to) {
    return null;
  }

  return { from, to };
}

function getMentionDeleteOffsets(
  spec: unknown,
): MentionDeleteDecorationSpec["mentionDelete"] | null {
  if (!spec || typeof spec !== "object") return null;

  const mentionDelete = (spec as MentionDeleteDecorationSpec).mentionDelete;
  if (!mentionDelete || typeof mentionDelete !== "object") return null;

  const { fromOffset, toOffset } = mentionDelete;
  if (!Number.isInteger(fromOffset) || !Number.isInteger(toOffset)) {
    return null;
  }

  return mentionDelete;
}
