import { Extension } from "@tiptap/core";
import {
  Plugin,
  PluginKey,
  TextSelection,
  type EditorState,
  type Transaction,
} from "@tiptap/pm/state";
import { Decoration, DecorationSet, type EditorView } from "@tiptap/pm/view";

export const mentionHighlightKey = new PluginKey("mentionHighlight");

/**
 * TipTap extension that applies inline `mention-chip` decorations
 * to `@Name` and `#channel-name` patterns in the document.
 *
 * Accepts `names` (display names) and `channelNames` storage options.
 * On every doc update the plugin scans text nodes and decorates matches.
 *
 * Agent mentions are treated as atomic for caret placement: the cursor
 * cannot rest inside `@AgentName` (which would break the chip when typing).
 * Arrow keys and backspace/delete also hop/delete the whole token.
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
        appendTransaction(_transactions, _oldState, newState) {
          return snapSelectionOutOfAgentMentions(
            newState,
            extension.storage.agentNames,
          );
        },
        props: {
          decorations(state) {
            return this.getState(state) ?? DecorationSet.empty;
          },
          handleClick(view, pos) {
            return snapViewSelectionToAgentMentionEdge(
              view,
              pos,
              extension.storage.agentNames,
            );
          },
          handleKeyDown(view, event) {
            return handleAgentMentionKeyDown(
              view,
              event,
              extension.storage.agentNames,
            );
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
      "mention-chip",
    );
    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      agentMentionPatterns,
      "mention-chip agent-mention-highlight",
      { hideMentionPrefix: true },
    );
    addMatchesForPatterns(
      decorations,
      node.text,
      pos,
      channelPatterns,
      "mention-chip",
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
  options?: { hideMentionPrefix?: boolean },
) {
  for (const pattern of patterns) {
    pattern.lastIndex = 0;
    let match: RegExpExecArray | null = pattern.exec(text);
    while (match !== null) {
      const from = position + match.index;
      const to = from + match[0].length;
      if (options?.hideMentionPrefix && match[0].startsWith("@")) {
        decorations.push(
          Decoration.inline(from, from + 1, {
            class: "agent-mention-at-hidden",
            spellcheck: "false",
          }),
        );
        decorations.push(
          Decoration.inline(from + 1, to, {
            class: className,
            spellcheck: "false",
          }),
        );
      } else {
        decorations.push(
          Decoration.inline(from, to, {
            class: className,
            spellcheck: "false",
          }),
        );
      }
      match = pattern.exec(text);
    }
  }
}

type MentionRange = { from: number; to: number };

/**
 * Locate `@AgentName` ranges in a ProseMirror doc for agent display names.
 * Exported for unit tests.
 */
export function findAgentMentionRanges(
  doc: EditorState["doc"],
  agentNames: readonly string[],
): MentionRange[] {
  if (agentNames.length === 0) return [];

  const patterns = buildHighlightPatterns([...agentNames], []);
  const ranges: MentionRange[] = [];

  doc.descendants((node, pos) => {
    if (!node.isText || !node.text) return;
    for (const match of findHighlightMatches(node.text, patterns)) {
      ranges.push({ from: pos + match.from, to: pos + match.to });
    }
  });

  return ranges;
}

/**
 * Snap a caret position out of an agent-mention interior.
 * Prefers the nearer edge; ties go to the end (after the chip).
 * Exported for unit tests.
 */
export function snapPosOutOfAgentMention(
  pos: number,
  ranges: readonly MentionRange[],
): number {
  for (const range of ranges) {
    if (pos > range.from && pos < range.to) {
      const toStart = pos - range.from;
      const toEnd = range.to - pos;
      return toStart < toEnd ? range.from : range.to;
    }
  }
  return pos;
}

function snapSelectionOutOfAgentMentions(
  state: EditorState,
  agentNames: readonly string[],
): Transaction | null {
  const { from, to, empty } = state.selection;
  if (!empty || from !== to) return null;

  const ranges = findAgentMentionRanges(state.doc, agentNames);
  const snapped = snapPosOutOfAgentMention(from, ranges);
  if (snapped === from) return null;

  return state.tr.setSelection(TextSelection.create(state.doc, snapped));
}

function snapViewSelectionToAgentMentionEdge(
  view: EditorView,
  pos: number,
  agentNames: readonly string[],
): boolean {
  const ranges = findAgentMentionRanges(view.state.doc, agentNames);
  const snapped = snapPosOutOfAgentMention(pos, ranges);
  if (snapped === pos) return false;

  view.dispatch(
    view.state.tr.setSelection(TextSelection.create(view.state.doc, snapped)),
  );
  return true;
}

function handleAgentMentionKeyDown(
  view: EditorView,
  event: KeyboardEvent,
  agentNames: readonly string[],
): boolean {
  if (event.altKey || event.metaKey || event.ctrlKey) return false;

  const ranges = findAgentMentionRanges(view.state.doc, agentNames);
  if (ranges.length === 0) return false;

  const { from, empty } = view.state.selection;
  if (!empty) return false;

  if (event.key === "ArrowLeft" && !event.shiftKey) {
    const range = ranges.find((candidate) => candidate.to === from);
    if (!range) return false;
    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, range.from),
      ),
    );
    return true;
  }

  if (event.key === "ArrowRight" && !event.shiftKey) {
    const range = ranges.find((candidate) => candidate.from === from);
    if (!range) return false;
    view.dispatch(
      view.state.tr.setSelection(
        TextSelection.create(view.state.doc, range.to),
      ),
    );
    return true;
  }

  if (event.key === "Backspace") {
    const range = ranges.find((candidate) => candidate.to === from);
    if (!range) return false;
    view.dispatch(view.state.tr.delete(range.from, range.to));
    return true;
  }

  if (event.key === "Delete") {
    const range = ranges.find((candidate) => candidate.from === from);
    if (!range) return false;
    view.dispatch(view.state.tr.delete(range.from, range.to));
    return true;
  }

  return false;
}
