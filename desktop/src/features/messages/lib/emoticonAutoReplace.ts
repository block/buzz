import { Extension, textInputRule } from "@tiptap/core";

import { escapeRegExp } from "@/shared/lib/mentionPattern";

import { EMOTICON_MAP } from "./emoticonMap";

/**
 * Build the `find` pattern for one emoticon: it must sit at a word boundary
 * — preceded by whitespace or the start of the input — so the rule only
 * fires on a standalone emoticon, not mid-word (e.g. `path:)`, or a
 * `:shortcode:` still being typed like `:Dance`). The capturing group holds
 * just the emoticon; `textInputRule` uses match[1] to leave the preceding
 * whitespace/start untouched and only replace that inner range.
 */
export function buildEmoticonFindPattern(ascii: string): RegExp {
  return new RegExp(`(?:^|\\s)(${escapeRegExp(ascii)})$`);
}

/**
 * Auto-replaces standalone ASCII emoticons (`:)`, `:P`, `<3`, ...) with their
 * unicode emoji equivalent the instant the user finishes typing them — same
 * instant "convert on completion" behavior as the known-`:shortcode:` input
 * rule in customEmojiNode.ts.
 *
 * Two things this deliberately does NOT do:
 * - Fire inside code blocks or inline code: Tiptap's input-rule plugin
 *   already skips both (see `run()` in @tiptap/core's InputRule.ts).
 * - Fire on pasted text: input rules hook the editor's `handleTextInput`
 *   prop, which only runs for real typed keystrokes (and IME composition
 *   end) — paste goes through ProseMirror's `handlePaste`/slice-insertion
 *   path instead, which never calls `handleTextInput`.
 */
export const EmoticonAutoReplace = Extension.create({
  name: "emoticonAutoReplace",

  addInputRules() {
    return Object.entries(EMOTICON_MAP).map(([ascii, emoji]) =>
      textInputRule({
        find: buildEmoticonFindPattern(ascii),
        replace: emoji,
      }),
    );
  },
});
