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

export type MentionCaretSettlement = {
  arm: (pos: number) => void;
  peek: () => number | null;
  cancel: () => void;
};

export function createMentionCaretSettlement(): MentionCaretSettlement {
  let pos: number | null = null;
  return {
    arm(nextPos: number) {
      pos = nextPos;
    },
    peek() {
      return pos;
    },
    cancel() {
      pos = null;
    },
  };
}

/**
 * Whether to move an empty caret from `from` to `next` after a mention
 * trailing space. Settlement is per editor: autocomplete arms it, and
 * ArrowLeft/click cancel it so we do not steal an intentional caret.
 *
 * Only an armed settlement may advance the caret. Advancing on any
 * document change instead walked the caret across the separator on every
 * keystroke, so typing `@name` before existing text interleaved spaces
 * into the draft (`hello @q uworld`).
 */
export function shouldAdvanceMentionCaret({
  from,
  next,
  settling,
}: {
  from: number;
  next: number;
  settling: boolean;
}): boolean {
  return next !== from && settling;
}

export type MentionTextInsertion = {
  insertAt: number;
  text: string;
};

const SPACE_RUN = /^[ \u00A0]+$/;
const OUTER_SPACES = /^[ \u00A0]+|[ \u00A0]+$/g;

/**
 * Position just after the trailing space of the mention token that `pos` is
 * adjacent to, or `null` when `pos` is nowhere near one.
 *
 * `pos` may sit at the token end (before the space) or already past the
 * space: when Chromium rewrites the whitespace run around the caret it
 * anchors the replacement at either edge, and both mean the same boundary.
 */
function mentionTrailingSpaceBoundary(
  doc: ProseMirrorNode,
  pos: number,
  names: readonly string[] = [],
): number | null {
  const afterSpace = selectionAfterMentionTrailingSpace(doc, pos, names);
  if (afterSpace !== pos) return afterSpace;
  if (
    pos > 0 &&
    selectionAfterMentionTrailingSpace(doc, pos - 1, names) === pos
  ) {
    return pos;
  }
  return null;
}

/**
 * Where (and what) to insert when typed text arrives at the trailing space
 * after an `@name` / `#channel` token.
 *
 * - Caret on the space: insert after it, so the next keystroke lands after
 *   the token (`@bobhello` fix).
 * - Whitespace replaced next to that space: keep every space the document
 *   already has and insert only the typed characters after the token's
 *   trailing space.
 *
 * The second rule matters because typing between the mention's trailing
 * space and a pre-existing draft space makes Chromium re-emit the whole
 * whitespace run — `replace("  " -> " a")`, usually with a non-breaking
 * space, and anchored at either edge of the run. Applying any of those
 * verbatim deletes the draft's space (`hello @bob abcworld`).
 *
 * Only whitespace is ever redirected, and only while autocomplete is
 * settling — a window in which the user cannot have selected anything,
 * because a selection cancels settlement. So a replacement arriving here
 * is the browser normalizing whitespace, never an intentional delete, and
 * preserving the document's spaces is the whole invariant. Recognizing one
 * specific rewrite shape instead is what left the draft space exposed.
 */
export function insertionForMentionTextInput(
  doc: ProseMirrorNode,
  from: number,
  to: number,
  text: string,
  names: readonly string[] = [],
): MentionTextInsertion | null {
  if (from === to) {
    const next = selectionAfterMentionTrailingSpace(doc, from, names);
    return next === from ? null : { insertAt: next, text };
  }
  const boundary = mentionTrailingSpaceBoundary(doc, from, names);
  if (boundary === null) return null;
  if (!SPACE_RUN.test(doc.textBetween(from, to, "\n", "\0"))) return null;
  return { insertAt: boundary, text: text.replace(OUTER_SPACES, "") };
}

/**
 * Redirect chip-edge typing only while autocomplete is settling. After a
 * deliberate ArrowLeft or chip click, honor the caret so `x` lands in the
 * token (`@bobx`) instead of after the space (`@bob x`).
 */
export function mentionTextInputInsertion(
  doc: ProseMirrorNode,
  from: number,
  to: number,
  text: string,
  settling: boolean,
  names: readonly string[] = [],
): MentionTextInsertion | null {
  if (!settling) return null;
  return insertionForMentionTextInput(doc, from, to, text, names);
}

/** Caret just after a mention trailing space: ArrowLeft lands on the token end. */
export function positionAfterArrowLeftThroughMentionSpace(
  doc: ProseMirrorNode,
  from: number,
  names: readonly string[] = [],
): number | null {
  if (from <= 0) return null;
  const chipEnd = from - 1;
  if (selectionAfterMentionTrailingSpace(doc, chipEnd, names) === from) {
    return chipEnd;
  }
  return null;
}

export function setDomCaretAtPos(
  view: {
    domAtPos: (pos: number) => { node: Node; offset: number };
    root: Document | ShadowRoot;
  },
  pos: number,
): void {
  if (typeof document === "undefined") return;
  let mapped: { node: Node; offset: number };
  try {
    mapped = view.domAtPos(pos);
  } catch {
    return;
  }
  const range = document.createRange();
  try {
    range.setStart(mapped.node, mapped.offset);
  } catch {
    return;
  }
  range.collapse(true);
  const root = view.root;
  const selection =
    "getSelection" in root && typeof root.getSelection === "function"
      ? root.getSelection()
      : window.getSelection();
  if (!selection) return;
  selection.removeAllRanges();
  selection.addRange(range);
}

export function reassertMentionCaretAfterFocus(view: {
  state: {
    doc: ProseMirrorNode;
    selection: { empty: boolean; from: number };
    tr: Transaction;
  };
  dispatch: (tr: Transaction) => void;
  domAtPos: (pos: number) => { node: Node; offset: number };
  root: Document | ShadowRoot;
}): void {
  if (!view.state.selection.empty) return;
  const from = view.state.selection.from;
  const next = selectionAfterMentionTrailingSpace(view.state.doc, from);
  if (next !== from) {
    view.dispatch(
      view.state.tr.setSelection(TextSelection.create(view.state.doc, next)),
    );
  }
  setDomCaretAtPos(view, view.state.selection.from);
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
  settleCaret = true,
): void {
  const storage = mentionHighlightStorage(editor);
  // Autocomplete provides a literal label, including spaces and disambiguators.
  const mentionInsert = /(?:^|[\s(])([@#])([^@#\r\n]+) $/.exec(text);
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
  if (settleCaret) tr.setMeta(mentionHighlightKey, true);
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
    const settlement = createMentionCaretSettlement();
    const knownNames = () => [
      ...extension.storage.names,
      ...extension.storage.agentNames,
      ...extension.storage.channelNames,
    ];

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
            if (
              tr.getMeta(mentionHighlightKey) &&
              tr.selection.empty &&
              (tr.docChanged || settlement.peek() !== null)
            ) {
              settlement.arm(
                selectionAfterMentionTrailingSpace(
                  tr.doc,
                  tr.selection.from,
                  knownNames(),
                ),
              );
            }

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
          if (!newState.selection.empty) {
            settlement.cancel();
            return null;
          }
          const from = newState.selection.from;
          const next = selectionAfterMentionTrailingSpace(
            newState.doc,
            from,
            knownNames(),
          );
          if (
            !shouldAdvanceMentionCaret({
              from,
              next,
              settling: settlement.peek() !== null,
            })
          ) {
            return null;
          }
          return newState.tr.setSelection(
            TextSelection.create(newState.doc, next),
          );
        },
        view() {
          let applying = false;
          return {
            update(view) {
              if (applying || settlement.peek() === null) return;
              if (!view.state.selection.empty) {
                settlement.cancel();
                return;
              }
              const from = view.state.selection.from;
              const next = selectionAfterMentionTrailingSpace(
                view.state.doc,
                from,
                knownNames(),
              );
              if (next !== from) {
                applying = true;
                try {
                  view.dispatch(
                    view.state.tr.setSelection(
                      TextSelection.create(view.state.doc, next),
                    ),
                  );
                } finally {
                  applying = false;
                }
              }
              // Highlight refreshes can land after the user has moved focus to
              // a popover. Keep settlement armed for the next keystroke, but
              // never drag DOM selection back into an unfocused composer.
              if (view.hasFocus()) {
                setDomCaretAtPos(view, view.state.selection.from);
              }
            },
            destroy() {
              settlement.cancel();
            },
          };
        },
        props: {
          decorations(state) {
            return this.getState(state) ?? DecorationSet.empty;
          },
          handleTextInput(view, from, to, text) {
            const insertion = mentionTextInputInsertion(
              view.state.doc,
              from,
              to,
              text,
              settlement.peek() !== null,
              knownNames(),
            );
            if (insertion == null) {
              settlement.cancel();
              return false;
            }
            const tr = view.state.tr.insertText(
              insertion.text,
              insertion.insertAt,
            );
            const caret = tr.mapping.map(insertion.insertAt, 1);
            tr.setSelection(TextSelection.create(tr.doc, caret));
            view.dispatch(tr);
            settlement.cancel();
            setDomCaretAtPos(view, caret);
            return true;
          },
          handleKeyDown(view, event) {
            if (
              event.key === "ArrowRight" ||
              event.key === "ArrowUp" ||
              event.key === "ArrowDown" ||
              event.key === "Home" ||
              event.key === "End"
            ) {
              settlement.cancel();
              return false;
            }
            if (event.key !== "ArrowLeft" || !view.state.selection.empty) {
              return false;
            }
            settlement.cancel();
            const chipEnd = positionAfterArrowLeftThroughMentionSpace(
              view.state.doc,
              view.state.selection.from,
              knownNames(),
            );
            if (chipEnd == null) return false;
            view.dispatch(
              view.state.tr.setSelection(
                TextSelection.create(view.state.doc, chipEnd),
              ),
            );
            setDomCaretAtPos(view, chipEnd);
            return true;
          },
          handleClick(view, pos, event) {
            const target = event.target;
            const onChip =
              target instanceof Element &&
              Boolean(target.closest(".mention-chip"));
            settlement.cancel();
            if (!onChip) return false;
            const chipEnd = positionAfterArrowLeftThroughMentionSpace(
              view.state.doc,
              pos,
              knownNames(),
            );
            if (chipEnd == null) return false;
            view.dispatch(
              view.state.tr.setSelection(
                TextSelection.create(view.state.doc, chipEnd),
              ),
            );
            setDomCaretAtPos(view, chipEnd);
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
  names: readonly string[] = [],
): number {
  if (pos < 0 || pos >= doc.content.size) return pos;
  const nextChar = doc.textBetween(pos, pos + 1, "\n", "\0");
  if (nextChar !== " ") return pos;
  // Use registered full labels as well as the legacy single-token fallback.
  // A multi-word name's internal spaces are not trailing separators.
  const lookbehind = Math.min(
    pos,
    names.reduce((max, name) => Math.max(max, name.length + 2), 80),
  );
  const before = doc.textBetween(pos - lookbehind, pos, "\n", "\0");
  const lower = before.toLowerCase();
  // Don't treat the space inside a registered multi-word label as the end of
  // a single-word token, even when the browser maps the caret there.
  const internalSpace = names.some((name) => {
    const token = name.toLowerCase();
    for (
      let space = token.indexOf(" ");
      space >= 0;
      space = token.indexOf(" ", space + 1)
    ) {
      const start = before.length - space - 1;
      if (start < 0 || (start > 0 && !/[\s(]/.test(before[start - 1])))
        continue;
      if (before[start] !== "@" && before[start] !== "#") continue;
      if (lower.slice(start + 1) !== token.slice(0, space)) continue;
      if (
        doc
          .textBetween(
            pos,
            Math.min(doc.content.size, pos + name.length - space),
            "\n",
            "\0",
          )
          .toLowerCase() === token.slice(space)
      )
        return true;
    }
    return false;
  });
  if (internalSpace) return pos;
  const knownBoundary = names.some((name) => {
    const start = before.length - name.length - 1;
    return (
      start >= 0 &&
      (start === 0 || /[\s(]/.test(before[start - 1])) &&
      (before[start] === "@" || before[start] === "#") &&
      lower.slice(start + 1) === name.toLowerCase()
    );
  });
  if (!knownBoundary && !/(?:^|[\s(])[@#][^\s]+$/.test(before)) return pos;
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
