import type * as React from "react";

import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";
import { ChannelComposerActivityAccessory } from "@/features/channels/ui/ChannelComposerActivityAccessory";
import {
  WelcomeComposerGuidanceLayer,
  type WelcomeComposerBannerState,
} from "@/features/channels/ui/WelcomeComposerBanner";
import { ComposerDockBackdrop } from "@/features/messages/ui/ComposerDockBackdrop";
import { ComposerUploadProgressOverlay } from "@/features/messages/ui/ComposerUploadProgressOverlay";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import type {
  MessageComposerEditTarget,
  MessageComposerProps,
} from "@/features/messages/ui/MessageComposer.types";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import type { TimeoutState } from "@/features/moderation/lib/timeoutStore";
import { ComposerTimeoutBanner } from "@/features/moderation/ui/ComposerTimeoutBanner";
import { cn } from "@/shared/lib/cn";
import type { useProjectedThreadComposer } from "./useProjectedThreadComposer";

type ChannelMainComposerDockProps = {
  activeChannel: ChannelPaneProps["activeChannel"];
  activityAgents: NonNullable<ChannelPaneProps["activityAgents"]>;
  autoSendDraftKey: ChannelPaneProps["autoSendDraftKey"];
  composerRef: React.Ref<HTMLDivElement>;
  composerWorkingBotPubkeys: string[];
  currentPubkey: ChannelPaneProps["currentPubkey"];
  directMessageIntroDisplayName: string | null;
  handleAutoSubmitComplete: () => void;
  handleDismissWelcomeBanner: () => void;
  handleEditLastOwnMainMessage: () => boolean;
  handleSendMessage: MessageComposerProps["onSend"];
  hasComposerBottomActivity: boolean;
  isActiveWelcomeChannel: boolean;
  isComposerDisabled: boolean;
  isModerationDmChannel: boolean;
  isSending: boolean;
  mainComposerMedia: MediaUploadController;
  mainEditTarget: MessageComposerEditTarget | null;
  onAttachmentAcceptanceChange: MessageComposerProps["onAttachmentAcceptanceChange"];
  onCancelEdit: ChannelPaneProps["onCancelEdit"];
  onEditSave: ChannelPaneProps["onEditSave"];
  onOpenAgentSession: ChannelPaneProps["onOpenAgentSession"];
  openAgentSessionPubkey: ChannelPaneProps["openAgentSessionPubkey"];
  prepareDmSendChannel: MessageComposerProps["onPrepareSendChannel"];
  profiles: ChannelPaneProps["profiles"];
  projectedThreadComposer: ReturnType<typeof useProjectedThreadComposer>;
  recentMentionPubkeys: MessageComposerProps["recentMentionPubkeys"];
  setMainDeferredEditPending: (pending: boolean) => void;
  timeoutState: TimeoutState;
  typingPubkeys: ChannelPaneProps["typingPubkeys"];
  welcomeComposerBannerState: WelcomeComposerBannerState;
  welcomeKickoffSettingUp: boolean;
  welcomeKickoffStage: React.ReactNode;
};

function getMainComposerPlaceholder({
  activeChannel,
  directMessageIntroDisplayName,
  isModerationDmChannel,
  isReplyingToProjectedThread,
  timeoutActive,
}: {
  activeChannel: ChannelPaneProps["activeChannel"];
  directMessageIntroDisplayName: string | null;
  isModerationDmChannel: boolean;
  isReplyingToProjectedThread: boolean;
  timeoutActive: boolean;
}) {
  if (isReplyingToProjectedThread) return undefined;
  if (timeoutActive) return "You're timed out by community moderators.";
  if (isModerationDmChannel) return "This channel is read-only.";
  if (activeChannel?.archivedAt) return "Archived channels are read-only.";
  if (activeChannel?.channelType === "forum") {
    return "Forum posting is not wired in this pass.";
  }
  if (!activeChannel) return "Select a channel";
  if (activeChannel.channelType === "dm" && directMessageIntroDisplayName) {
    return `Message ${directMessageIntroDisplayName}`;
  }
  return `Message #${activeChannel.name}`;
}

export function ChannelMainComposerDock({
  activeChannel,
  activityAgents,
  autoSendDraftKey,
  composerRef,
  composerWorkingBotPubkeys,
  currentPubkey,
  directMessageIntroDisplayName,
  handleAutoSubmitComplete,
  handleDismissWelcomeBanner,
  handleEditLastOwnMainMessage,
  handleSendMessage,
  hasComposerBottomActivity,
  isActiveWelcomeChannel,
  isComposerDisabled,
  isModerationDmChannel,
  isSending,
  mainComposerMedia,
  mainEditTarget,
  onAttachmentAcceptanceChange,
  onCancelEdit,
  onEditSave,
  onOpenAgentSession,
  openAgentSessionPubkey,
  prepareDmSendChannel,
  profiles,
  projectedThreadComposer,
  recentMentionPubkeys,
  setMainDeferredEditPending,
  timeoutState,
  typingPubkeys,
  welcomeComposerBannerState,
  welcomeKickoffSettingUp,
  welcomeKickoffStage,
}: ChannelMainComposerDockProps) {
  return (
    <div
      className="pointer-events-none absolute inset-x-0 bottom-0 z-40 isolate before:absolute before:inset-x-0 before:bottom-0 before:-z-10 before:h-24 before:bg-gradient-to-b before:from-transparent before:to-background before:content-[''] after:absolute after:inset-x-0 after:bottom-0 after:-z-10 after:h-12 after:bg-background after:content-['']"
      data-testid="channel-composer-overlay"
      ref={composerRef}
    >
      <ComposerUploadProgressOverlay />
      <div
        className={cn(
          "composer-dock composer-overlay-corner-masks relative pointer-events-auto",
          hasComposerBottomActivity && "composer-dock--with-activity",
        )}
      >
        {isActiveWelcomeChannel && !timeoutState.active ? (
          <WelcomeComposerGuidanceLayer
            onDismiss={handleDismissWelcomeBanner}
            settingUp={welcomeKickoffSettingUp}
            state={welcomeComposerBannerState}
          >
            {welcomeKickoffStage}
          </WelcomeComposerGuidanceLayer>
        ) : null}
        {timeoutState.active ? (
          <ComposerTimeoutBanner expiresAtMs={timeoutState.expiresAtMs} />
        ) : null}
        <ComposerDockBackdrop gutterClassName="inset-x-5" />
        <MessageComposer
          autoSubmitDraftKey={autoSendDraftKey}
          channelId={activeChannel?.id ?? null}
          channelName={activeChannel?.name ?? "channel"}
          channelType={activeChannel?.channelType ?? null}
          containerClassName="px-5 pb-0"
          disabled={isComposerDisabled}
          editTarget={mainEditTarget}
          isSending={isSending}
          layoutMode="dock"
          mediaController={mainComposerMedia}
          onAttachmentAcceptanceChange={onAttachmentAcceptanceChange}
          onAutoSubmitComplete={handleAutoSubmitComplete}
          onCancelEdit={onCancelEdit}
          onCancelReply={
            projectedThreadComposer.composerTarget
              ? projectedThreadComposer.cancelReply
              : undefined
          }
          onCaptureSendContext={projectedThreadComposer.captureSendContext}
          onDeferredEditPendingChange={setMainDeferredEditPending}
          onEditLastOwnMessage={handleEditLastOwnMainMessage}
          onEditSave={onEditSave}
          onPrepareSendChannel={
            activeChannel?.channelType === "dm"
              ? prepareDmSendChannel
              : undefined
          }
          onSend={handleSendMessage}
          placeholder={getMainComposerPlaceholder({
            activeChannel,
            directMessageIntroDisplayName,
            isModerationDmChannel,
            isReplyingToProjectedThread:
              projectedThreadComposer.composerTarget !== null,
            timeoutActive: timeoutState.active,
          })}
          profiles={profiles}
          recentMentionPubkeys={recentMentionPubkeys}
          replyTarget={projectedThreadComposer.composerTarget}
          showBackgroundUploadProgress={false}
          showTopBorder={false}
          typingParentEventId={projectedThreadComposer.target?.id ?? null}
          typingRootEventId={projectedThreadComposer.rootId}
        />
        <ChannelComposerActivityAccessory
          agents={activityAgents}
          channel={activeChannel}
          currentPubkey={currentPubkey}
          onOpenAgentSession={onOpenAgentSession}
          openAgentSessionPubkey={openAgentSessionPubkey}
          profiles={profiles}
          typingPubkeys={typingPubkeys}
          visible={hasComposerBottomActivity}
          workingBotPubkeys={composerWorkingBotPubkeys}
        />
      </div>
    </div>
  );
}
