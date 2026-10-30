import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import type { EditorView } from "@tiptap/pm/view";

import { collectEmojiShortcodes } from "@/shared/lib/customEmojiTags";
import type { EmojiNameResolver } from "@/shared/lib/customEmojiTags";

import { CUSTOM_EMOJI_NODE_NAME, EMOJI_TOKEN_PATTERN } from "./customEmojiNode";

type TokenReplacement = {
  from: number;
  to: number;
  shortcode: string;
  src: string;
};

const WORD_CHAR = /[A-Za-z0-9_]/;

/**
 * Text `:name:` tokens in `doc` whose name resolves, outside code. Boundary
 * rule matches the markdown rule: a token glued to a preceding word character
 * stays text.
 */
export function findResolvedEmojiTokens(
  doc: ProseMirrorNode,
  resolveUrl: (shortcode: string) => string | undefined,
): TokenReplacement[] {
  const found: TokenReplacement[] = [];
  doc.descendants((node, pos, parent) => {
    if (!node.isText || !node.text || parent?.type.spec.code) return;
    if (node.marks.some((mark) => mark.type.name === "code")) return;
    for (const match of node.text.matchAll(
      new RegExp(EMOJI_TOKEN_PATTERN, "g"),
    )) {
      const index = match.index ?? 0;
      if (index > 0 && WORD_CHAR.test(node.text[index - 1] ?? "")) continue;
      const shortcode = match[0].slice(1, -1).toLowerCase();
      const src = resolveUrl(shortcode);
      if (src === undefined) continue;
      found.push({
        from: pos + index,
        to: pos + index + match[0].length,
        shortcode,
        src,
      });
    }
  });
  return found;
}

/**
 * After a paste, resolve every `:name:` in the pasted text with one batched
 * lookup, then turn the resolved tokens still in the document into emoji
 * nodes. Positions are read from the document at that moment, so edits made
 * while the lookup ran cannot misplace a replacement. A failed lookup leaves
 * the text as typed; the send path looks the names up again.
 */
export async function resolvePastedEmoji(
  view: EditorView,
  pastedText: string,
  resolver: EmojiNameResolver,
  resolveUrl: (shortcode: string) => string | undefined,
): Promise<void> {
  const names = collectEmojiShortcodes(pastedText).filter(
    (name) => resolveUrl(name) === undefined,
  );
  if (names.length === 0) return;
  const { failed } = await resolver.resolve(names);
  if (failed || view.isDestroyed) return;
  const nodeType = view.state.schema.nodes[CUSTOM_EMOJI_NODE_NAME];
  if (!nodeType) return;
  const replacements = findResolvedEmojiTokens(view.state.doc, resolveUrl);
  if (replacements.length === 0) return;
  let tr = view.state.tr;
  for (const { from, to, shortcode, src } of replacements.reverse()) {
    tr = tr.replaceWith(from, to, nodeType.create({ shortcode, src }));
  }
  view.dispatch(tr.setMeta("addToHistory", false));
}
