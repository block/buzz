import type { LinkSelectionInfo } from "./useRichTextEditor";

export type LinkEditorInitialFocus = "text" | "url";

export function getLinkEditorInitialFocus(
  info: LinkSelectionInfo,
): LinkEditorInitialFocus {
  const isSelectedTextLinkInsert =
    info.href.length === 0 &&
    info.text.trim().length > 0 &&
    info.from !== info.to;

  return isSelectedTextLinkInsert ? "url" : "text";
}
