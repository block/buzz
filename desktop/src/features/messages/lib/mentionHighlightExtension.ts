import { Extension } from "@tiptap/core";
import {
  Plugin,
  PluginKey,
  type EditorState,
  type Transaction,
} from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

export const mentionHighlightKey = new PluginKey("mentionHighlight");

type AgentMentionDecorationSpec = {
  mentionKind: "agent";
  segment: "prefix" | "label";
};

/**
 * Return the authoritative decorated agent-mention range whose final
 * character is immediately before `to`, or null when no exact range exists.
 *
 * Agent mention rendering uses two adjacent decorations so WebKit can hide
 * the `@` prefix independently from the visible label. Both segments must be
 * present at their exact mapped positions; plain matching text is never
 * sufficient.
 */
export function findAgentMentionDecorationEndingAt(
  state: EditorState,
  to: number,
): { from: number; to: number } | null {
  const decorations = mentionHighlightKey.getState(state);
  if (!decorations || to <= 1) return null;

  const agentSegments = decorations.find(
    0,
    state.doc.content.size,
    (spec: AgentMentionDecorationSpec) => spec.mentionKind === "agent",
  );
  const label = agentSegments.find(
    (decoration: Decoration) =>
      decoration.to === to && decoration.spec.segment === "label",
  );
  if (!label || label.from <= 1) return null;

  const prefixFrom = label.from - 1;
  const hasExactPrefix = agentSegments.some(
    (decoration: Decoration) =>
      decoration.from === prefixFrom &&
      decoration.to === label.from &&
      decoration.spec.segment === "prefix",
  );
  if (!hasExactPrefix) return null;

  if (state.doc.textBetween(prefixFrom, label.from) !== "@") return null;
  return { from: prefixFrom, to };
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
          Decoration.inline(
            from,
            from + 1,
            {
              class: "agent-mention-at-hidden",
              spellcheck: "false",
            },
            {
              mentionKind: "agent",
              segment: "prefix",
            } satisfies AgentMentionDecorationSpec,
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
            {
              mentionKind: "agent",
              segment: "label",
            } satisfies AgentMentionDecorationSpec,
          ),
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
