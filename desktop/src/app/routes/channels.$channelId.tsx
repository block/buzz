import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ChannelRouteSearch = {
  messageId?: string;
  threadRootId?: string;
};

function validateChannelSearch(
  search: Record<string, unknown>,
): ChannelRouteSearch {
  return {
    messageId:
      typeof search.messageId === "string" && search.messageId.length > 0
        ? search.messageId
        : undefined,
    threadRootId:
      typeof search.threadRootId === "string" && search.threadRootId.length > 0
        ? search.threadRootId
        : undefined,
  };
}

export const Route = createFileRoute("/channels/$channelId")({
  validateSearch: validateChannelSearch,
  component: ChannelRouteComponent,
});

const ChannelRouteScreen = React.lazy(async () => {
  const module = await import("./ChannelRouteScreen");
  return { default: module.ChannelRouteScreen };
});

function ChannelRouteComponent() {
  const { channelId } = Route.useParams();
  const search = Route.useSearch();

  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="channel" />}
    >
      <ChannelRouteScreen
        channelId={channelId}
        selectedPostId={null}
        targetMessageId={search.messageId ?? null}
        targetReplyId={null}
        targetThreadRootId={search.threadRootId ?? null}
      />
    </React.Suspense>
  );
}
