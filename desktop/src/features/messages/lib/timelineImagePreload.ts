import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { TimelineMessage } from "../types";
import { parseImetaTags } from "./parseImeta";

const MARKDOWN_IMAGE_RE = /!\[[^\]]*\]\(([^)\s]+)(?:\s+["'][^"']*["'])?\)/g;

/**
 * Return every image URL a mounted timeline row can request. Keeping this
 * projection independent of row rendering lets a virtualized timeline warm the
 * browser cache before the row enters Virtua's mounted range.
 */
export function timelineImageUrls(message: TimelineMessage): string[] {
  const urls = new Set<string>();
  const add = (url: string | null | undefined) => {
    if (url) urls.add(rewriteRelayUrl(url));
  };

  add(message.avatarUrl);

  for (const match of message.body.matchAll(MARKDOWN_IMAGE_RE)) {
    add(match[1]);
  }

  if (message.tags) {
    for (const entry of parseImetaTags(message.tags).values()) {
      if (entry.m?.startsWith("image/")) add(entry.url);
      // Video poster frames are images too, and otherwise arrive only when the
      // virtualized row mounts its player.
      add(entry.image);
      add(entry.thumb);
    }
    for (const tag of message.tags) {
      if (tag[0] === "emoji") add(tag[2]);
    }
  }

  for (const reaction of message.reactions ?? []) add(reaction.emojiUrl);
  return [...urls];
}

export type TimelineImagePreloadState = {
  activeImages: Set<HTMLImageElement>;
  requestedUrls: Set<string>;
};

/** Start all image requests now, independently of Virtua row mounting. */
export function preloadTimelineImages(
  messages: readonly TimelineMessage[],
  state: TimelineImagePreloadState,
): void {
  for (const message of messages) {
    for (const url of timelineImageUrls(message)) {
      if (state.requestedUrls.has(url)) continue;
      state.requestedUrls.add(url);
      const image = new Image();
      state.activeImages.add(image);
      const release = () => state.activeImages.delete(image);
      image.addEventListener("load", release, { once: true });
      image.addEventListener("error", release, { once: true });
      image.src = url;
    }
  }
}
