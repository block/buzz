import * as React from "react";

import { Markdown } from "@/shared/ui/markdown";
import type { MarkdownProps } from "@/shared/ui/markdown/types";
import { useOpenVideoReviewAt } from "@/shared/ui/VideoReviewNavigation";
import { parseVideoReviewTimecode } from "@/shared/ui/videoReviewTimecode";
import {
  VideoReviewTimecodeButton,
  VideoReviewTimecodeChip,
} from "@/shared/ui/VideoReviewTimecodeButton";

type VideoReviewCommentMarkdownProps = Omit<
  MarkdownProps,
  "leadingInlineContent"
> & {
  videoReviewCommentRootId?: string;
};

/** Renders a video-review timecode inside the comment's first Markdown line. */
export function VideoReviewCommentMarkdown({
  content,
  interactive = true,
  videoReviewCommentRootId,
  ...markdownProps
}: VideoReviewCommentMarkdownProps) {
  const openVideoReviewAt = useOpenVideoReviewAt();
  const reviewTimecode = React.useMemo(
    () => (videoReviewCommentRootId ? parseVideoReviewTimecode(content) : null),
    [content, videoReviewCommentRootId],
  );
  const handleTimecodeClick = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (reviewTimecode && videoReviewCommentRootId) {
        openVideoReviewAt?.(videoReviewCommentRootId, reviewTimecode.seconds);
      }
    },
    [openVideoReviewAt, reviewTimecode, videoReviewCommentRootId],
  );
  const leadingInlineContent = React.useMemo(() => {
    if (!reviewTimecode) return undefined;

    const timecode =
      interactive && openVideoReviewAt ? (
        <VideoReviewTimecodeButton
          surface="message"
          timecode={reviewTimecode.timecode}
          onClick={handleTimecodeClick}
        />
      ) : (
        <VideoReviewTimecodeChip
          surface="message"
          timecode={reviewTimecode.timecode}
        />
      );
    return <>{timecode} </>;
  }, [handleTimecodeClick, interactive, openVideoReviewAt, reviewTimecode]);

  if (!reviewTimecode) {
    return (
      <Markdown
        {...markdownProps}
        content={content}
        interactive={interactive}
      />
    );
  }

  return (
    <Markdown
      {...markdownProps}
      content={reviewTimecode.text || "\u200B"}
      interactive={interactive}
      leadingInlineContent={leadingInlineContent}
    />
  );
}
