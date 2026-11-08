import * as React from "react";
import { createFileRoute, useLocation } from "@tanstack/react-router";

import { selectSearchHighlightRouteState } from "@/app/routes/searchHighlightRouteState";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ForumPostRouteSearch = {
  replyId?: string;
};

function validateForumPostSearch(
  search: Record<string, unknown>,
): ForumPostRouteSearch {
  return {
    replyId:
      typeof search.replyId === "string" && search.replyId.length > 0
        ? search.replyId
        : undefined,
  };
}

export const Route = createFileRoute("/channels/$channelId/posts/$postId")({
  validateSearch: validateForumPostSearch,
  component: ForumPostRouteComponent,
});

const ChannelRouteScreen = React.lazy(async () => {
  const module = await import("./ChannelRouteScreen");
  return { default: module.ChannelRouteScreen };
});

function ForumPostRouteComponent() {
  usePreviewFeatureWarning("forum");
  const { channelId, postId } = Route.useParams();
  const search = Route.useSearch();
  const searchHighlight = useLocation({
    select: selectSearchHighlightRouteState,
  });

  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="forum" />}
    >
      <ChannelRouteScreen
        autoSendDraftKey={null}
        channelId={channelId}
        searchHighlight={searchHighlight}
        selectedPostId={postId}
        targetMessageId={null}
        targetReplyId={search.replyId ?? null}
        targetThreadRootId={null}
      />
    </React.Suspense>
  );
}
