export type EmojiListKeyAction = "next" | "prev" | "select" | "close";

type KeyLike = {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
};

/**
 * Map a key press to an action on a result list that keeps focus in a text
 * field. Space is never an action, with or without Shift: it types a space.
 * The composer list takes forward Tab as a selection; the picker panel leaves
 * Tab to the browser so focus can move on.
 */
export function emojiListKeyAction(
  event: KeyLike,
  options: { tabSelects: boolean },
): EmojiListKeyAction | null {
  const plain = !event.ctrlKey && !event.metaKey && !event.altKey;
  if (event.key === "ArrowDown") return "next";
  if (event.key === "ArrowUp") return "prev";
  if (event.key === "Escape") return "close";
  if (event.key === "Enter" && plain && !event.shiftKey) return "select";
  if (event.key === "Tab" && options.tabSelects && plain && !event.shiftKey) {
    return "select";
  }
  return null;
}
