import * as React from "react";

import { Markdown } from "@/shared/ui/markdown";
import type { MarkdownProps } from "@/shared/ui/markdown/types";
import { useOpenVideoReviewAt } from "@/shared/ui/VideoReviewNavigation";
import { parseVideoReviewTimecode } from "@/shared/ui/videoReviewTimecode";
import {
  VideoReviewTimecodeButton,
  VideoReviewTimecodeChip,
} from "@/shared/ui/VideoReviewTimecodeButton";

type VideoReviewCommentMarkdownProps = MarkdownProps & {
  videoReviewCommentRootId?: string;
};

/** Renders a video-review timecode inside the comment's first Markdown line. */
export function VideoReviewCommentMarkdown({
  content,
  interactive = true,
  leadingInlineContent: suppliedLeadingInlineContent,
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
    if (!reviewTimecode) return suppliedLeadingInlineContent;

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
    return (
      <>
        {suppliedLeadingInlineContent}
        {timecode}{" "}
      </>
    );
  }, [
    handleTimecodeClick,
    interactive,
    openVideoReviewAt,
    reviewTimecode,
    suppliedLeadingInlineContent,
  ]);

  return (
    <Markdown
      {...markdownProps}
      content={reviewTimecode ? reviewTimecode.text || "\u200B" : content}
      interactive={interactive}
      leadingInlineContent={leadingInlineContent}
    />
  );
}
