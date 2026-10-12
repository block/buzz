import * as React from "react";
import { createFileRoute, useLocation } from "@tanstack/react-router";

import { selectSearchHighlightRouteState } from "@/app/routes/searchHighlightRouteState";

import {
  parseProfilePanelTab,
  parseProfilePanelView,
  type ProfilePanelTab,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { HuddleStartingView } from "@/features/huddle/components/HuddleStartingView";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ChannelRouteSearch = {
  agentSession?: string;
  /**
   * When set, the composer on mount will auto-submit its loaded draft once,
   * then clear this param. Value is the draft key that was loaded so the
   * composer can verify it has the right draft before firing.
   */
  autoSend?: string;
  messageId?: string;
  profile?: string;
  profileTab?: ProfilePanelTab;
  profileView?: ProfilePanelView;
  thread?: string;
  threadRootId?: string;
};

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * Event-id params tolerate a query glued onto the value (e.g. `?thread=<id>?relay=wss://…` when `?relay=` was appended after the
 * hash instead of before it): keep only what precedes the first `?`, `&` or `#`, otherwise the relay rejects "Invalid event ID".
 */
function eventIdParam(value: unknown): string | undefined {
  const s = nonEmptyString(value);
  return s ? nonEmptyString(s.split(/[?&#]/, 1)[0]?.trim()) : undefined;
}

function validateChannelSearch(
  search: Record<string, unknown>,
): ChannelRouteSearch {
  return {
    agentSession: nonEmptyString(search.agentSession),
    autoSend: nonEmptyString(search.autoSend),
    messageId: eventIdParam(search.messageId),
    profile: nonEmptyString(search.profile),
    profileTab: parseProfilePanelTab(search.profileTab) ?? undefined,
    profileView: parseProfilePanelView(search.profileView) ?? undefined,
    thread: eventIdParam(search.thread),
    threadRootId: eventIdParam(search.threadRootId),
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
  const searchHighlight = useLocation({
    select: selectSearchHighlightRouteState,
  });
  const isHuddleTranscript = huddleWindowChannelId() !== null;

  return (
    <React.Suspense
      fallback={
        isHuddleTranscript ? (
          <HuddleStartingView />
        ) : (
          <ViewLoadingFallback includeHeader kind="channel" />
        )
      }
    >
      <ChannelRouteScreen
        autoSendDraftKey={search.autoSend ?? null}
        channelId={channelId}
        searchHighlight={searchHighlight}
        selectedPostId={null}
        targetMessageId={search.messageId ?? null}
        targetReplyId={null}
        targetThreadRootId={search.threadRootId ?? search.thread ?? null}
      />
    </React.Suspense>
  );
}
