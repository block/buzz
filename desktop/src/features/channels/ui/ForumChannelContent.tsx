import * as React from "react";

import {
  ForumView,
  UserProfilePanel,
} from "@/features/channels/ui/ChannelScreenLazyViews";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import type {
  ProfilePanelTab,
  ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import type { Channel } from "@/shared/api/types";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ForumChannelContentProps = {
  canResetPanelWidth: boolean;
  channel: Channel;
  currentPubkey?: string;
  header: React.ReactNode;
  onClosePost: () => void;
  onCloseProfilePanel: () => void;
  onOpenDm?: (pubkeys: string[]) => Promise<void> | void;
  onOpenProfilePanel: (pubkey: string) => void;
  onPanelResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onProfilePanelTabChange: (
    tab: ProfilePanelTab,
    options?: { replace?: boolean },
  ) => void;
  onProfilePanelViewChange: (
    view: ProfilePanelView,
    options?: { replace?: boolean },
  ) => void;
  onResetPanelWidth: () => void;
  onSelectPost: (postId: string) => void;
  panelWidthPx: number;
  profilePanelPubkey?: string | null;
  profilePanelTab: ProfilePanelTab;
  profilePanelView: ProfilePanelView;
  selectedPostId: string | null;
  targetReplyId: string | null;
};

/**
 * Forum-channel body for ChannelScreen: the post list/thread plus the
 * user-profile auxiliary pane. Forums replace ChannelPane (which hosts the
 * profile panel for message channels), so without this host, opening a
 * profile from a mention chip, avatar, or the members sidebar would set
 * state that never renders.
 */
export function ForumChannelContent({
  canResetPanelWidth,
  channel,
  currentPubkey,
  header,
  onClosePost,
  onCloseProfilePanel,
  onOpenDm,
  onOpenProfilePanel,
  onPanelResizeStart,
  onProfilePanelTabChange,
  onProfilePanelViewChange,
  onResetPanelWidth,
  onSelectPost,
  panelWidthPx,
  profilePanelPubkey,
  profilePanelTab,
  profilePanelView,
  selectedPostId,
  targetReplyId,
}: ForumChannelContentProps) {
  return (
    <>
      {header}
      <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
        <section
          aria-label="Forum posts"
          className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
        >
          <React.Suspense fallback={<ViewLoadingFallback kind="forum" />}>
            <ForumView
              channel={channel}
              currentPubkey={currentPubkey}
              onClosePost={onClosePost}
              onSelectPost={onSelectPost}
              selectedPostId={selectedPostId}
              targetReplyId={targetReplyId}
            />
          </React.Suspense>
        </section>
        {profilePanelPubkey ? (
          <RightAuxiliaryPane
            canResetWidth={canResetPanelWidth}
            onResetWidth={onResetPanelWidth}
            onResizeStart={onPanelResizeStart}
            testId="user-profile-panel"
            widthPx={panelWidthPx}
          >
            <React.Suspense fallback={null}>
              <UserProfilePanel
                callerChannelId={channel.id}
                currentPubkey={currentPubkey}
                isSinglePanelView={false}
                layout="split"
                onClose={onCloseProfilePanel}
                onOpenDm={onOpenDm}
                onOpenProfile={onOpenProfilePanel}
                onTabChange={onProfilePanelTabChange}
                onViewChange={onProfilePanelViewChange}
                pubkey={profilePanelPubkey}
                splitPaneClamp
                tab={profilePanelTab}
                view={profilePanelView}
                widthPx={panelWidthPx}
              />
            </React.Suspense>
          </RightAuxiliaryPane>
        ) : null}
      </div>
    </>
  );
}
