import * as React from "react";

import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

import { VideoPlayer, type VideoReviewContext } from "../VideoPlayer";
import type { ImetaEntry } from "./types";
import { aspectRatioFromDim } from "./utils";

/**
 * Video review context flows through React context instead of
 * `createMarkdownComponents` arguments. The component map must keep a stable
 * identity across re-renders: a new map means new element types, which makes
 * React unmount and remount every rendered node — including `<video>`
 * elements, killing playback (and any in-progress review comment draft)
 * whenever the timeline re-renders.
 */
export const VideoReviewMarkdownContext = React.createContext<
  VideoReviewContext | undefined
>(undefined);

export function MarkdownVideoPlayer({
  alt,
  entry,
  resolvedSrc,
  src,
}: {
  alt?: string;
  entry?: ImetaEntry;
  resolvedSrc: string;
  src?: string;
}) {
  const videoReviewContext = React.useContext(VideoReviewMarkdownContext);
  // Look up poster frame from imeta tags (NIP-71 `image` field).
  // Fall back to `thumb` for compatibility with older events.
  const posterUrl = entry?.image ?? entry?.thumb;
  const resolvedPoster = posterUrl ? rewriteRelayUrl(posterUrl) : undefined;
  const resolvedReviewContext = React.useMemo(
    () =>
      videoReviewContext
        ? {
            ...videoReviewContext,
            title:
              videoReviewContext.title ?? entry?.filename ?? alt ?? "Video",
          }
        : undefined,
    [alt, entry?.filename, videoReviewContext],
  );

  return (
    <VideoPlayer
      src={resolvedSrc}
      aspectRatio={aspectRatioFromDim(entry?.dim)}
      poster={resolvedPoster}
      durationSeconds={entry?.duration}
      reviewKey={src ?? resolvedSrc}
      reviewContext={resolvedReviewContext}
    />
  );
}
