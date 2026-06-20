import * as React from "react";

import { customEmojiFromTags } from "@/shared/api/customEmoji";
import { isEmojiOnlyMessage } from "@/shared/lib/emojiOnly";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/**
 * Derive custom-emoji rendering info for a message body from its raw event
 * tags. Shared by channel and inbox message rows so NIP-30 `:shortcode:`
 * emoji (and the emoji-only large-render treatment) behave identically
 * everywhere a message body is shown.
 */
export function useMessageEmoji(
  body: string,
  tags: ReadonlyArray<ReadonlyArray<string>> | undefined,
): { customEmoji: CustomEmoji[] | undefined; emojiOnly: boolean } {
  const customEmoji = React.useMemo(
    () => (tags ? customEmojiFromTags(tags) : undefined),
    [tags],
  );
  const emojiOnly = React.useMemo(
    () => isEmojiOnlyMessage(body, customEmoji),
    [body, customEmoji],
  );
  return { customEmoji, emojiOnly };
}
