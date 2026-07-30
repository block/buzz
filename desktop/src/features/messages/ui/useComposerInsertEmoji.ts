import * as React from "react";

import type { Editor } from "@tiptap/react";

import { CUSTOM_EMOJI_NODE_NAME } from "@/features/messages/lib/customEmojiNode";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

type InsertEmojiParams = {
  editor: Editor | null;
  customEmoji: CustomEmoji[];
  /** Side effects to run after an insert (close the picker, clear mentions). */
  onAfterInsert: () => void;
};

/**
 * Insert an emoji into the composer from the toolbar emoji picker.
 *
 * A `:shortcode:` for a known custom emoji becomes a selectable atom node (same
 * as the input rule / autocomplete) so it can be selected, copied, and deleted
 * as one unit; everything else (native unicode) inserts as plain content.
 */
export function useComposerInsertEmoji({
  editor,
  customEmoji,
  onAfterInsert,
}: InsertEmojiParams) {
  // Keep the post-insert side effects in a ref so the returned callback's
  // identity tracks only editor/customEmoji, exactly as it did inline.
  const onAfterInsertRef = React.useRef(onAfterInsert);
  onAfterInsertRef.current = onAfterInsert;

  return React.useCallback(
    (emoji: string) => {
      if (!editor) return;
      const match = /^:([^:\s]+):$/.exec(emoji);
      const shortcode = match?.[1]?.toLowerCase();
      const known =
        shortcode &&
        customEmoji.some((e) => e.shortcode.toLowerCase() === shortcode);
      if (known && shortcode) {
        editor
          .chain()
          .focus()
          .insertContent({
            type: CUSTOM_EMOJI_NODE_NAME,
            attrs: {
              shortcode,
              src:
                customEmoji.find((e) => e.shortcode.toLowerCase() === shortcode)
                  ?.url ?? "",
            },
          })
          .insertContent(" ")
          .run();
      } else {
        editor.chain().focus().insertContent(emoji).run();
      }
      onAfterInsertRef.current();
    },
    [editor, customEmoji],
  );
}
