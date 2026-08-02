import { isMessageLink, parseMessageLink } from "./messageLink";
import type { ParsedMessageLink } from "./messageLink";
import { isRepoLink, parseRepoLink } from "@/features/projects/lib/repoLink";
import type { ParsedRepoLink } from "@/features/projects/lib/repoLink";

/**
 * Open a link the same way the rendered-message link path does:
 * Supported Buzz deep-links navigate in-app; everything else goes to the OS
 * opener. Mirrors `markdown.tsx`'s anchor renderer so the composer popover and
 * the rendered link behave identically.
 */
export function openPopoverLink(
  url: string,
  handlers: {
    openExternal: (url: string) => void;
    openMessageLink: (link: ParsedMessageLink) => void;
    openRepoLink: (link: ParsedRepoLink) => void;
  },
): void {
  if (isMessageLink(url)) {
    const parsed = parseMessageLink(url);
    if (parsed.ok) {
      handlers.openMessageLink(parsed.value);
      return;
    }
  }
  if (isRepoLink(url)) {
    const parsed = parseRepoLink(url);
    if (parsed.ok) {
      handlers.openRepoLink(parsed.value);
      return;
    }
  }
  handlers.openExternal(url);
}
