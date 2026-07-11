import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { TimelineMessage } from "../types";
import { parseImetaTags } from "./parseImeta";

/**
 * Return non-message-media image URLs worth warming before a virtualized row
 * mounts. Inline image attachments deliberately stay out of this projection:
 * their native-lazy thumbnail must load before the full-resolution request.
 */
export function timelineImageUrls(message: TimelineMessage): string[] {
  const urls = new Set<string>();
  const add = (url: string | null | undefined) => {
    if (url) urls.add(rewriteRelayUrl(url));
  };

  add(message.avatarUrl);

  if (message.tags) {
    for (const entry of parseImetaTags(message.tags).values()) {
      // Video poster frames are not part of the progressive image path.
      if (entry.m?.startsWith("video/")) {
        add(entry.image);
        add(entry.thumb);
      }
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
