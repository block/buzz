import { Extension } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import {
  Plugin,
  PluginKey,
  TextSelection,
  type Transaction,
} from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import {
  inlineChipIconClasses,
  MENTION_CHIP_BASE_CLASSES,
} from "@/shared/ui/mentionChip";

export const mentionHighlightKey = new PluginKey("mentionHighlight");

const MENTION_CARET_FENCE_MS = 750;
let mentionCaretFenceUntil = 0;

export function armMentionCaretFence(): void {
  mentionCaretFenceUntil = Date.now() + MENTION_CARET_FENCE_MS;
}

export function clearMentionCaretFence(): void {
  mentionCaretFenceUntil = 0;
}

export function mentionCaretFenceActive(now = Date.now()): boolean {
  return now < mentionCaretFenceUntil;
}

/**
 * Whether to move an empty caret from `from` to `next` after a mention
 * trailing space. Autocomplete arms a short fence so DOM selection remaps
 * (no doc/meta transaction) cannot put the next keystroke before the space.
 * Arrow keys after the fence expires are left alone.
 */
export function shouldAdvanceMentionCaret({
  from,
  next,
  fenceActive,
  rebuilt,
}: {
  from: number;
  next: number;
  fenceActive: boolean;
  rebuilt: boolean;
}): boolean {
  return next !== from && (fenceActive || rebuilt);
}

export type MentionHighlightStorage = {
  names: string[];
  agentNames: string[];
  channelNames: string[];
};

function sameNameList(current: string[], next: string[]): boolean {
  return (
    current.length === next.length &&
    current.every((name, index) => name === next[index])
  );
}

export function assignMentionHighlightNames(
  storage: MentionHighlightStorage,
  names: string[],
  agentNames: string[],
  channelNames: string[],
): boolean {
  if (
    sameNameList(storage.names, names) &&
    sameNameList(storage.agentNames, agentNames) &&
    sameNameList(storage.channelNames, channelNames)
  ) {
    return false;
  }
  storage.names = names;
  storage.agentNames = agentNames;
  storage.channelNames = channelNames;
  return true;
}

export function mentionHighlightStorage(editor: {
  storage: object;
}): MentionHighlightStorage | undefined {
  if (!("mentionHighlight" in editor.storage)) return undefined;
  return editor.storage.mentionHighlight as MentionHighlightStorage;
}

export function settleAutocompleteMentionInsert(
  editor: { storage: object },
  tr: Transaction,
  text: string,
): void {
  const storage = mentionHighlightStorage(editor);
  const mentionInsert = /(?:^|[\s(])([@#])([^\s]+) $/.exec(text);
  if (!mentionInsert) return;
  const prefix = mentionInsert[1];
  const label = mentionInsert[2];
  if (storage) {
    const known = [
      ...storage.names,
      ...storage.agentNames,
      ...storage.channelNames,
    ];
    if (!known.some((name) => name.toLowerCase() === label.toLowerCase())) {
      if (prefix === "#") {
        storage.channelNames = [...storage.channelNames, label];
      } else {
        storage.names = [...storage.names, label];
      }
    }
  }
  tr.setMeta(mentionHighlightKey, true);
  armMentionCaretFence();
}

export function syncMentionHighlightFromProps(
  editor: {
    storage: object;
    state: { tr: Transaction };
    view: { dispatch: (tr: Transaction) => void };
  },
  names: string[] | undefined,
  agentNames: string[] | undefined,
  channelNames: string[] | undefined,
): void {
  const storage = mentionHighlightStorage(editor);
  if (
    !storage ||
    !assignMentionHighlightNames(
      storage,
      names ?? [],
      agentNames ?? [],
      channelNames ?? [],
    )
  ) {
    return;
  }
  editor.view.dispatch(editor.state.tr.setMeta(mentionHighlightKey, true));
}

/**
 * TipTap extension that applies inline `mention-chip` decorations
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
        appendTransaction(transactions, _oldState, newState) {
          if (!newState.selection.empty) return null;
          const rebuilt = transactions.some(
            (tr) => tr.docChanged || tr.getMeta(mentionHighlightKey),
          );
          const from = newState.selection.from;
          const next = selectionAfterMentionTrailingSpace(newState.doc, from);
          if (
            !shouldAdvanceMentionCaret({
              from,
              next,
              fenceActive: mentionCaretFenceActive(),
              rebuilt,
            })
          ) {
            return null;
          }
          return newState.tr.setSelection(
            TextSelection.create(newState.doc, next),
          );
        },
        props: {
          decorations(state) {
            return this.getState(state) ?? DecorationSet.empty;
          },
          handleTextInput(view, from, to, text) {
            if (from !== to) return false;
            const next = selectionAfterMentionTrailingSpace(
              view.state.doc,
              from,
            );
            if (next === from) return false;
            const tr = view.state.tr.insertText(text, next);
            tr.setSelection(
              TextSelection.create(tr.doc, tr.mapping.map(next, 1)),
            );
            view.dispatch(tr);
            armMentionCaretFence();
            return true;
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
      new RegExp(
        `(?:^|(?<=[\\s(]))@(${escapedNames.join("|")})(?=\\W|$)`,
        "gi",
      ),
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
 * If `pos` sits at the end of an `@name` / `#channel` token and the next
 * character is a space, return the position after that space.
 *
 * Autocomplete inserts `@Name ` then chip decorations wrap the token. The
 * browser can map the caret back to the chip edge, so the next keystroke
 * lands before the space (`@quinnhello`). Callers use this to keep typing
 * after the token.
 */
export function selectionAfterMentionTrailingSpace(
  doc: ProseMirrorNode,
  pos: number,
): number {
  if (pos < 0 || pos >= doc.content.size) return pos;
  const nextChar = doc.textBetween(pos, pos + 1, "\n", "\0");
  if (nextChar !== " ") return pos;
  const lookbehind = Math.min(pos, 80);
  const before = doc.textBetween(pos - lookbehind, pos, "\n", "\0");
  if (!/(?:^|[\s(])[@#][^\s]+$/.test(before)) return pos;
  return pos + 1;
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
      `${MENTION_CHIP_BASE_CLASSES} ${inlineChipIconClasses("human")}`,
      { hidePrefix: true },
    );
    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      agentMentionPatterns,
      `${MENTION_CHIP_BASE_CLASSES} ${inlineChipIconClasses("agent")}`,
      { hidePrefix: true },
    );
    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      channelPatterns,
      `${MENTION_CHIP_BASE_CLASSES} ${inlineChipIconClasses("channel")}`,
      { hidePrefix: true },
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
  options?: { hidePrefix?: boolean },
) {
  for (const pattern of patterns) {
    pattern.lastIndex = 0;
    let match: RegExpExecArray | null = pattern.exec(text);
    while (match !== null) {
      const from = position + match.index;
      const to = from + match[0].length;
      const outsideEnd = { inclusiveEnd: false };
      if (options?.hidePrefix && /^[@#]/.test(match[0])) {
        decorations.push(
          Decoration.inline(
            from,
            from + 1,
            {
              class: "mention-prefix-hidden",
              spellcheck: "false",
            },
            outsideEnd,
          ),
        );
        decorations.push(
          Decoration.inline(
            from + 1,
            to,
            {
              class: className,
              spellcheck: "false",
            },
            outsideEnd,
          ),
        );
      } else {
        decorations.push(
          Decoration.inline(
            from,
            to,
            {
              class: className,
              spellcheck: "false",
            },
            outsideEnd,
          ),
        );
      }
      match = pattern.exec(text);
    }
  }
}
