import type { Components } from "react-markdown";
import { i18n } from "@/i18n";

/** Keep parsed element caches separate for each render variant and language. */
export function createMarkdownComponentCache(
  factory: (
    interactive: boolean,
    mediaInset: boolean,
    blockCode: boolean,
  ) => Components,
) {
  const entries = new Map<
    string,
    { components: Components; variant: string }
  >();
  return (
    interactive: boolean,
    leadingInlineContent: boolean,
    mediaInset: boolean,
    blockCode: boolean,
  ) => {
    const variant = `8:${i18n.resolvedLanguage}:${interactive ? "i" : ""}${leadingInlineContent ? "l" : ""}${mediaInset ? "m" : ""}${blockCode ? "c" : ""}`;
    let entry = entries.get(variant);
    if (!entry) {
      entry = {
        components: factory(interactive, mediaInset, blockCode),
        variant,
      };
      entries.set(variant, entry);
    }
    return entry;
  };
}
