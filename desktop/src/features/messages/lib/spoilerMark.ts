import { Mark, mergeAttributes } from "@tiptap/core";

export const SPOILER_MARK_NAME = "spoiler";
const SPOILER_MARKDOWN_RULE = "buzz_spoiler";
const SPOILER_OPEN_TOKEN = "buzz_spoiler_open";
const SPOILER_CLOSE_TOKEN = "buzz_spoiler_close";
const PIPE_CHAR = 0x7c;

export function registerSpoilerMarkdownIt(
  // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
  md: any,
): void {
  if (md.renderer.rules[SPOILER_OPEN_TOKEN]) return;

  // biome-ignore lint/suspicious/noExplicitAny: markdown-it state/silent
  const rule = (state: any, silent: boolean): boolean => {
    const start = state.pos;
    if (
      state.src.charCodeAt(start) !== PIPE_CHAR ||
      state.src.charCodeAt(start + 1) !== PIPE_CHAR
    ) {
      return false;
    }

    const contentStart = start + 2;
    const contentEnd = findClosingDelimiter(
      state.src,
      contentStart,
      state.posMax,
    );
    if (contentEnd <= contentStart) return false;

    if (!silent) {
      const previousPosMax = state.posMax;
      state.push(SPOILER_OPEN_TOKEN, "span", 1);
      state.pos = contentStart;
      state.posMax = contentEnd;
      state.md.inline.tokenize(state);
      state.push(SPOILER_CLOSE_TOKEN, "span", -1);
      state.posMax = previousPosMax;
    }

    state.pos = contentEnd + 2;
    return true;
  };

  // Composer parsing is intentionally inline-only. Block delimiter spoilers
  // are rendered by remarkSpoilers as receive-only interop, but are not part of
  // the composer WYSIWYG surface.
  md.inline.ruler.before("emphasis", SPOILER_MARKDOWN_RULE, rule);
  md.renderer.rules[SPOILER_OPEN_TOKEN] = () =>
    '<span data-spoiler="" class="buzz-spoiler">';
  md.renderer.rules[SPOILER_CLOSE_TOKEN] = () => "</span>";
}

/**
 * Match the first closing delimiter, mirroring Discord-style spoiler parsing.
 * Inner `||` text closes the spoiler instead of nesting or scanning greedily.
 */
export function findClosingDelimiter(
  source: string,
  start: number,
  end: number,
): number {
  for (let index = start; index < end - 1; index++) {
    if (
      source.charCodeAt(index) === PIPE_CHAR &&
      source.charCodeAt(index + 1) === PIPE_CHAR
    ) {
      return index;
    }
  }
  return -1;
}

export const SpoilerMark = Mark.create({
  name: SPOILER_MARK_NAME,

  parseHTML() {
    return [{ tag: "span[data-spoiler]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-spoiler": "",
        class: "buzz-spoiler",
      }),
      0,
    ];
  },

  addStorage() {
    return {
      markdown: {
        serialize: {
          open: "||",
          close: "||",
          expelEnclosingWhitespace: true,
        },
        parse: {
          // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
          setup(_md: any) {
            registerSpoilerMarkdownIt(_md);
          },
        },
      },
    };
  },
});
