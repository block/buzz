import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  useChannelMembersQuery,
  useJoinChannelMutation,
  useSetCanvasMutation,
} from "@/features/channels/hooks";
import {
  appendCanvasBoardCard,
  type CanvasBoardCard,
  type CanvasBoardCardDraft,
  reorderCanvasBoardCard,
  updateCanvasBoardCard,
} from "@/features/channels/lib/canvasBoard";
import { useActiveChannelHeader } from "@/features/channels/useActiveChannelHeader";
import { ChannelBoard } from "@/features/channels/ui/ChannelBoard";
import { ChannelBoardCardEditorDialog } from "@/features/channels/ui/ChannelBoardCardEditorDialog";
import { ChannelManagementSheet } from "@/features/channels/ui/ChannelManagementSheet";
import { useChannelModerationCapabilities } from "@/features/channels/ui/ChannelManagementModerationActions";
import { ChannelScreenHeader } from "@/features/channels/ui/ChannelScreenHeader";
import { MembersSidebar } from "@/features/channels/ui/MembersSidebar";
import { useChannelViewMode } from "@/features/channels/ui/ChannelViewModeContext";
import { useCommunities } from "@/features/communities/useCommunities";
import type { CanvasResponse, Channel } from "@/shared/api/types";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_SHORT,
} from "@/shared/lib/relayError";
import { normalizePubkey } from "@/shared/lib/pubkey";

type ChannelBoardScreenProps = {
  canvas: CanvasResponse | undefined;
  canvasError: unknown;
  canvasLoading: boolean;
  channel: Channel;
  currentPubkey?: string;
};

type CardEditorState = {
  baseContent: string;
  card: CanvasBoardCard | null;
};

export function ChannelBoardScreen({
  canvas,
  canvasError,
  canvasLoading,
  channel,
  currentPubkey,
}: ChannelBoardScreenProps) {
  const { activeCommunity } = useCommunities();
  const queryClient = useQueryClient();
  const channelViewMode = useChannelViewMode();
  const membersQuery = useChannelMembersQuery(channel.id);
  const joinChannelMutation = useJoinChannelMutation(channel.id);
  const setCanvasMutation = useSetCanvasMutation(channel.id);
  const members = membersQuery.data ?? [];
  const agentCount = members.filter(
    (member) => member.isAgent || member.role === "bot",
  ).length;
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  const [isChannelManagementOpen, setIsChannelManagementOpen] =
    React.useState(false);
  const [isAddBotOpen, setIsAddBotOpen] = React.useState(false);
  const [cardEditor, setCardEditor] = React.useState<CardEditorState | null>(
    null,
  );
  const [actionErrorMessage, setActionErrorMessage] = React.useState<
    string | null
  >(null);
  const {
    activeChannelEphemeralDisplay,
    activeChannelTitle,
    activeDmAvatarUrl,
    activeDmHeaderParticipants,
    activeDmPresenceStatus,
  } = useActiveChannelHeader(channel, currentPubkey);
  const canvasErrorMessage =
    canvasError instanceof Error
      ? isRelayUnreachableError(canvasError)
        ? RELAY_UNREACHABLE_SHORT
        : canvasError.message
      : undefined;
  const { canManageChannel } = useChannelModerationCapabilities(
    membersQuery.data,
    currentPubkey,
    true,
  );
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const selfMember = members.find(
    (member) =>
      normalizedCurrentPubkey !== null &&
      normalizePubkey(member.pubkey) === normalizedCurrentPubkey,
  );
  const canEditBoard =
    canManageChannel &&
    selfMember !== undefined &&
    channel.archivedAt === null &&
    channel.channelType !== "dm";
  const sourceContent = canvas?.content ?? "";
  const isBoardSaving = setCanvasMutation.isPending;

  function openCreateCard() {
    setActionErrorMessage(null);
    setCardEditor({ baseContent: sourceContent, card: null });
  }

  function openEditCard(card: CanvasBoardCard) {
    setActionErrorMessage(null);
    setCardEditor({ baseContent: sourceContent, card });
  }

  async function persistCanvasContent(
    nextContent: string,
    successMessage: string,
  ) {
    setActionErrorMessage(null);
    const canvasQueryKey = ["channel-canvas", channel.id] as const;
    const previousCanvas =
      queryClient.getQueryData<CanvasResponse>(canvasQueryKey);
    queryClient.setQueryData<CanvasResponse>(canvasQueryKey, {
      author: currentPubkey ?? previousCanvas?.author ?? null,
      content: nextContent,
      updatedAt: Math.floor(Date.now() / 1_000),
    });
    try {
      await setCanvasMutation.mutateAsync(nextContent);
      toast.success(successMessage);
    } catch (error) {
      queryClient.setQueryData<CanvasResponse | undefined>(
        canvasQueryKey,
        (current) =>
          current?.content === nextContent ? previousCanvas : current,
      );
      const message =
        error instanceof Error ? error.message : "Couldn’t save the board.";
      setActionErrorMessage(message);
      throw error;
    }
  }

  async function handleSaveCard(draft: CanvasBoardCardDraft) {
    if (!cardEditor) {
      return;
    }
    if (sourceContent !== cardEditor.baseContent) {
      const message =
        "This board changed while the card was open. Close and reopen the card before saving.";
      setActionErrorMessage(message);
      throw new Error(message);
    }

    const nextContent = cardEditor.card
      ? updateCanvasBoardCard(sourceContent, cardEditor.card.id, draft)
      : appendCanvasBoardCard(sourceContent, draft);
    if (nextContent === null) {
      const message =
        "This card is no longer on the board. Reopen it and try again.";
      setActionErrorMessage(message);
      throw new Error(message);
    }

    await persistCanvasContent(
      nextContent,
      cardEditor.card ? "Card updated" : "Card created",
    );
    setCardEditor(null);
  }

  async function handleMoveCard(activeCardId: string, overCardId: string) {
    const nextContent = reorderCanvasBoardCard(
      sourceContent,
      activeCardId,
      overCardId,
    );
    if (nextContent === null || nextContent === sourceContent) {
      return;
    }

    try {
      await persistCanvasContent(nextContent, "Card order saved");
    } catch {
      // The board restores the relay-backed order and shows the error inline.
    }
  }

  return (
    <React.Fragment>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <ChannelScreenHeader
          activeChannel={channel}
          activeChannelEphemeralDisplay={activeChannelEphemeralDisplay}
          activeChannelTitle={activeChannelTitle}
          activeDmAvatarUrl={activeDmAvatarUrl}
          activeDmHeaderParticipants={activeDmHeaderParticipants}
          activeDmPresenceStatus={activeDmPresenceStatus}
          currentPubkey={currentPubkey}
          isAddBotOpen={isAddBotOpen}
          isJoining={joinChannelMutation.isPending}
          onAddBotOpenChange={setIsAddBotOpen}
          onJoinChannel={joinChannelMutation.mutateAsync}
          onManageChannel={() => setIsChannelManagementOpen(true)}
          onToggleMembers={() => setIsMembersSidebarOpen((open) => !open)}
        />
        <ChannelBoard
          actionErrorMessage={actionErrorMessage ?? undefined}
          agentCount={agentCount}
          author={canvas?.author ?? null}
          canEdit={canEditBoard}
          channelName={activeChannelTitle}
          content={sourceContent}
          errorMessage={canvasErrorMessage}
          isLoading={canvasLoading}
          isSaving={isBoardSaving}
          memberCount={members.length || channel.memberCount}
          onCreateCard={openCreateCard}
          onEditCard={openEditCard}
          onManageBoard={() => setIsChannelManagementOpen(true)}
          onMoveCard={(activeCardId, overCardId) => {
            void handleMoveCard(activeCardId, overCardId);
          }}
          onOpenMembers={() => setIsMembersSidebarOpen(true)}
          onOpenStream={() => channelViewMode.onModeChange("stream")}
          updatedAt={canvas?.updatedAt ?? null}
        />
      </div>

      <ChannelBoardCardEditorDialog
        card={cardEditor?.card ?? null}
        errorMessage={actionErrorMessage}
        isSaving={isBoardSaving}
        onOpenChange={(open) => {
          if (!open) {
            setCardEditor(null);
            setActionErrorMessage(null);
          }
        }}
        onSave={handleSaveCard}
        open={cardEditor !== null}
      />

      <ChannelManagementSheet
        channel={channel}
        currentPubkey={currentPubkey}
        onOpenChange={setIsChannelManagementOpen}
        open={isChannelManagementOpen}
      />
      <MembersSidebar
        channel={channel}
        currentPubkey={currentPubkey}
        onOpenChange={setIsMembersSidebarOpen}
        open={isMembersSidebarOpen}
        relayUrl={activeCommunity?.relayUrl}
      />
    </React.Fragment>
  );
}
