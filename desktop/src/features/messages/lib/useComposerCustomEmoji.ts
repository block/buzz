import type { Editor } from "@tiptap/react";
import * as React from "react";

import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { CUSTOM_EMOJI_NODE_NAME, CustomEmojiNode } from "./customEmojiNode";

/**
 * React wiring for the composer's custom-emoji atom node, kept out of
 * `useRichTextEditor` so that file stays focused on generic editor setup and
 * `customEmojiNode.ts` stays a pure (testable) ProseMirror primitive.
 *
 * Returns:
 *  - `extension`: the configured `CustomEmojiNode` to register on the editor.
 *    Its `resolveUrl`/`shortcodes` read an always-current ref, so a single
 *    configuration tracks the live emoji set without re-creating the editor.
 *  - `syncEmojiSrc(editor)`: re-resolves the `src` attr on existing nodes in
 *    the doc when the set changes (e.g. an emoji's image was just published).
 *    Call from an effect keyed on the emoji set.
 */
export function useComposerCustomEmoji(customEmoji: CustomEmoji[] | undefined) {
  const ref = React.useRef<CustomEmoji[]>(customEmoji ?? []);
  ref.current = customEmoji ?? [];

  const resolveUrl = React.useCallback(
    (shortcode: string): string | undefined =>
      ref.current.find(
        (e) => e.shortcode.toLowerCase() === shortcode.toLowerCase(),
      )?.url,
    [],
  );

  const extension = React.useMemo(
    () =>
      CustomEmojiNode.configure({
        resolveUrl,
        shortcodes: () => ref.current.map((e) => e.shortcode),
      }),
    [resolveUrl],
  );

  const syncEmojiSrc = React.useCallback(
    (editor: Editor) => {
      const urlByShortcode = new Map(
        (customEmoji ?? []).map((e) => [e.shortcode.toLowerCase(), e.url]),
      );
      const { state } = editor;
      let tr = state.tr;
      let changed = false;
      state.doc.descendants((node, pos) => {
        if (node.type.name !== CUSTOM_EMOJI_NODE_NAME) return;
        const shortcode = String(node.attrs.shortcode ?? "").toLowerCase();
        const nextSrc =
          urlByShortcode.get(shortcode) ?? String(node.attrs.src ?? "");
        if (nextSrc !== node.attrs.src) {
          tr = tr.setNodeAttribute(pos, "src", nextSrc);
          changed = true;
        }
      });
      if (changed) {
        // Passive re-resolve, not a user edit → keep it off the undo stack.
        editor.view.dispatch(tr.setMeta("addToHistory", false));
      }
    },
    [customEmoji],
  );

  return { extension, resolveUrl, syncEmojiSrc };
}
