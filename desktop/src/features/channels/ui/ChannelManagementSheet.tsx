import {
  Archive,
  BookOpenText,
  Copy,
  DoorClosed,
  DoorOpen,
  FileText,
  Fingerprint,
  Eye,
  Lock,
  MessageSquare,
  Pencil,
  Radio,
  Type,
  Users,
  Zap,
} from "lucide-react";
import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { toast } from "sonner";

import {
  useArchiveChannelMutation,
  useCanvasQuery,
  useChannelDetailsQuery,
  useChannelMembersQuery,
  useDeleteChannelMutation,
  useJoinChannelMutation,
  useLeaveChannelMutation,
  useSetChannelPurposeMutation,
  useSetChannelTopicMutation,
  useUnarchiveChannelMutation,
  useUpdateChannelMutation,
} from "@/features/channels/hooks";
import { compareMembersByRole } from "@/features/channels/lib/memberUtils";
import {
  DEFAULT_EPHEMERAL_TTL_SECONDS,
  formatTtlDuration,
  parseTtlDuration,
} from "@/features/channels/lib/ephemeralChannel";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  AuxiliaryPanelBody,
  AuxiliaryPanelContext,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
  type AuxiliaryPanelMode,
  getAuxiliaryPanelMode,
} from "@/shared/layout/AuxiliaryPanel";
import { useScrollBoundaryLock } from "@/shared/hooks/useScrollBoundaryLock";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_ENTER_MOTION_CLASS,
  PANEL_OVERLAY_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { ChannelCanvas } from "./ChannelCanvas";
import {
  ChannelHero,
  ChannelQuickAction,
  CopyFieldRow,
  FieldGroup,
  getMarkdownPreviewText,
  InfoFieldRow,
  IngressRow,
  NarrativeField,
  NarrativeGroup,
  ToggleRow,
} from "./ChannelManagementSheetRows";
import {
  ChannelManagementModerationActions,
  useChannelModerationCapabilities,
} from "./ChannelManagementModerationActions";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type ChannelManagementSheetProps = {
  channel: Channel | null;
  animateSplitEnter?: boolean;
  currentPubkey?: string;
  layout?: "overlay" | "split";
  onDeleted?: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  transparentChrome?: boolean;
};

export function ChannelManagementSheet({
  animateSplitEnter = false,
  channel,
  currentPubkey,
  layout = "overlay",
  onDeleted,
  onOpenChange,
  open,
  transparentChrome = false,
}: ChannelManagementSheetProps) {
  const { isDark } = useTheme();
  const isSplitLayout = layout === "split";
  const auxiliaryPanelMode = getAuxiliaryPanelMode(
    isSplitLayout,
    !isSplitLayout,
  );
  const channelId = channel?.id ?? null;
  const detailsQuery = useChannelDetailsQuery(channelId, open);
  const membersQuery = useChannelMembersQuery(channelId, open);
  const canvasQuery = useCanvasQuery(channelId, channelId !== null && open);
  const updateChannelDetailsMutation = useUpdateChannelMutation(channelId);
  const setTopicMutation = useSetChannelTopicMutation(channelId);
  const setPurposeMutation = useSetChannelPurposeMutation(channelId);
  const archiveChannelMutation = useArchiveChannelMutation(channelId);
  const unarchiveChannelMutation = useUnarchiveChannelMutation(channelId);
  const deleteChannelMutation = useDeleteChannelMutation(channelId);
  const joinChannelMutation = useJoinChannelMutation(channelId);
  const leaveChannelMutation = useLeaveChannelMutation(channelId);

  const detail = detailsQuery.data ?? channel;
  const members = React.useMemo(() => {
    const currentMembers = membersQuery.data ?? [];
    return [...currentMembers].sort((left, right) =>
      compareMembersByRole(left, right, currentPubkey),
    );
  }, [currentPubkey, membersQuery.data]);
  const selfMember =
    members.find((member) => member.pubkey === currentPubkey) ?? null;
  const hasResolvedMembership = membersQuery.data !== undefined;

  const { canDeleteChannel, canManageChannel } =
    useChannelModerationCapabilities(membersQuery.data, currentPubkey, open);
  const canEditNarrative =
    canManageChannel && selfMember !== null && detail?.channelType !== "dm";
  const isArchived =
    detail?.archivedAt !== null && detail?.archivedAt !== undefined;
  const canJoin =
    hasResolvedMembership &&
    detail?.channelType !== "dm" &&
    detail?.visibility === "open" &&
    !isArchived &&
    selfMember === null;
  const canLeave =
    hasResolvedMembership &&
    detail?.channelType !== "dm" &&
    !isArchived &&
    selfMember !== null;
  const memberCount =
    members.length || detail?.memberCount || channel?.memberCount || 0;

  const [nameDraft, setNameDraft] = React.useState("");
  const [descriptionDraft, setDescriptionDraft] = React.useState("");
  const [topicDraft, setTopicDraft] = React.useState("");
  const [purposeDraft, setPurposeDraft] = React.useState("");
  const [isPrivateDraft, setIsPrivateDraft] = React.useState(false);
  const [isEphemeralDraft, setIsEphemeralDraft] = React.useState(false);
  const [ttlDraft, setTtlDraft] = React.useState("");
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = React.useState(false);
  const [isEditDialogOpen, setIsEditDialogOpen] = React.useState(false);
  const [activeView, setActiveView] = React.useState<"summary" | "canvas">(
    "summary",
  );

  // Sync drafts from server only when the sheet opens or the channel changes -
  // not on every background refetch, which would clobber in-flight edits.
  const syncedForRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!open) {
      // Reset on close so the next open re-syncs from server.
      syncedForRef.current = null;
      setIsDeleteDialogOpen(false);
      setIsEditDialogOpen(false);
      setActiveView("summary");
      return;
    }
    if (!detail) {
      return;
    }

    const key = detail.id;
    if (syncedForRef.current === key) {
      return;
    }
    syncedForRef.current = key;

    setNameDraft(detail.name);
    setDescriptionDraft(detail.description);
    setTopicDraft(detail.topic ?? "");
    setPurposeDraft(detail.purpose ?? "");
    setIsPrivateDraft(detail.visibility === "private");
    setIsEphemeralDraft(detail.ttlSeconds !== null);
    setTtlDraft(
      detail.ttlSeconds !== null ? formatTtlDuration(detail.ttlSeconds) : "",
    );
    setActiveView("summary");
  }, [detail, open]);

  if (!channel) {
    return null;
  }

  function handleDeleteDialogOpenChange(next: boolean) {
    deleteChannelMutation.reset();
    setIsDeleteDialogOpen(next);
  }

  async function handleDeleteChannel() {
    try {
      await deleteChannelMutation.mutateAsync();
      handleDeleteDialogOpenChange(false);
      onOpenChange(false);
      onDeleted?.();
    } catch {
      // The mutation error is rendered inline in the confirmation dialog.
    }
  }

  function handlePanelOpenChange(next: boolean) {
    if (!next) {
      handleDeleteDialogOpenChange(false);
    }

    onOpenChange(next);
  }

  // Parsed seconds for the ephemeral TTL field. `null` when the field is empty
  // or malformed; the form blocks saving on a non-empty malformed value.
  const parsedTtlSeconds = parseTtlDuration(ttlDraft);
  const ttlInvalid =
    isEphemeralDraft && ttlDraft.trim() !== "" && parsedTtlSeconds === null;

  const currentVisibility = detail?.visibility ?? channel.visibility;
  const currentTtlSeconds = detail?.ttlSeconds ?? null;
  const nextVisibility: "open" | "private" = isPrivateDraft
    ? "private"
    : "open";
  const nextTtlSeconds: number | null = isEphemeralDraft
    ? (parsedTtlSeconds ?? DEFAULT_EPHEMERAL_TTL_SECONDS)
    : null;
  const lifecycleDirty =
    nextVisibility !== currentVisibility ||
    nextTtlSeconds !== currentTtlSeconds;

  const resolvedChannel = detail ?? channel;
  const nameDirty = nameDraft.trim() !== resolvedChannel.name.trim();
  const descriptionDirty =
    descriptionDraft.trim() !== resolvedChannel.description.trim();
  const topicDirty = topicDraft.trim() !== (resolvedChannel.topic ?? "").trim();
  const purposeDirty =
    purposeDraft.trim() !== (resolvedChannel.purpose ?? "").trim();
  const isSavingChannelEdits =
    updateChannelDetailsMutation.isPending ||
    setTopicMutation.isPending ||
    setPurposeMutation.isPending;
  const hasChannelEditChanges =
    nameDirty ||
    descriptionDirty ||
    lifecycleDirty ||
    topicDirty ||
    purposeDirty;
  const canSaveChannelEdits =
    nameDraft.trim().length > 0 &&
    !ttlInvalid &&
    hasChannelEditChanges &&
    !isSavingChannelEdits;
  const canvasContent = canvasQuery.data?.content?.trim() ?? "";
  const hasCanvas = canvasContent.length > 0;
  const canvasPreview = hasCanvas
    ? getMarkdownPreviewText(canvasContent)
    : undefined;
  const canOpenCanvas = hasCanvas || canEditNarrative;

  async function handleSaveChannelEdits() {
    try {
      if (nameDirty || descriptionDirty || lifecycleDirty) {
        await updateChannelDetailsMutation.mutateAsync({
          description: descriptionDirty ? descriptionDraft.trim() : undefined,
          name: nameDirty ? nameDraft.trim() : undefined,
          ttlSeconds:
            nextTtlSeconds !== currentTtlSeconds ? nextTtlSeconds : undefined,
          visibility:
            lifecycleDirty && nextVisibility !== currentVisibility
              ? nextVisibility
              : undefined,
        });
      }

      if (topicDirty) {
        await setTopicMutation.mutateAsync({ topic: topicDraft.trim() });
      }

      if (purposeDirty) {
        await setPurposeMutation.mutateAsync({ purpose: purposeDraft.trim() });
      }

      setIsEditDialogOpen(false);
    } catch {
      // React Query stores mutation errors; keep the dialog open and render them.
    }
  }

  return (
    <DialogPrimitive.Root
      modal={!isSplitLayout}
      onOpenChange={handlePanelOpenChange}
      open={open}
    >
      {!isSplitLayout ? (
        <DialogPrimitive.Portal>
          <DialogPrimitive.Overlay asChild>
            <OverlayPanelBackdrop
              onClose={() => handlePanelOpenChange(false)}
            />
          </DialogPrimitive.Overlay>
        </DialogPrimitive.Portal>
      ) : null}
      {isSplitLayout ? (
        // No translucent backdrop-blur surface here: `backdrop-filter`
        // creates a stacking context that traps the z-40 panel header below
        // the shared z-30 header blur strip in split layout. The pane sits on
        // the opaque `bg-background` from PANEL_BASE_CLASS instead.
        <DialogPrimitive.Content
          className={cn(
            PANEL_BASE_CLASS,
            "h-full w-full cursor-default overflow-hidden border-l-0 p-0",
            animateSplitEnter && PANEL_ENTER_MOTION_CLASS,
          )}
          data-testid="channel-management-sheet"
          onEscapeKeyDown={(event) => event.preventDefault()}
          onInteractOutside={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
        >
          <ChannelManagementPanelContent
            activeView={activeView}
            archiveChannelMutation={archiveChannelMutation}
            canEditNarrative={canEditNarrative}
            canJoin={canJoin}
            canLeave={canLeave}
            canManageChannel={canManageChannel}
            canOpenCanvas={canOpenCanvas}
            canvasPreview={canvasPreview}
            canvasQuery={canvasQuery}
            channelId={channelId}
            deleteChannelMutation={deleteChannelMutation}
            detailsError={detailsQuery.error}
            handleDeleteChannel={handleDeleteChannel}
            handleDeleteDialogOpenChange={handleDeleteDialogOpenChange}
            isArchived={isArchived}
            isDark={isDark}
            isDeleteDialogOpen={isDeleteDialogOpen}
            canDeleteChannel={canDeleteChannel}
            mode={auxiliaryPanelMode}
            transparentChrome={transparentChrome}
            joinChannelMutation={joinChannelMutation}
            leaveChannelMutation={leaveChannelMutation}
            memberCount={memberCount}
            membersError={membersQuery.error}
            onOpenChange={handlePanelOpenChange}
            resolvedChannel={resolvedChannel}
            setActiveView={setActiveView}
            setIsEditDialogOpen={setIsEditDialogOpen}
            unarchiveChannelMutation={unarchiveChannelMutation}
          />
        </DialogPrimitive.Content>
      ) : (
        <DialogPrimitive.Portal>
          <DialogPrimitive.Content
            className={cn(
              PANEL_BASE_CLASS,
              PANEL_OVERLAY_CLASS,
              PANEL_ENTER_MOTION_CLASS,
              "w-[380px] cursor-default overflow-hidden p-0 data-[state=closed]:animate-out data-[state=closed]:slide-out-to-right data-[state=closed]:duration-200",
              isDark
                ? "bg-background/85 backdrop-blur-xl supports-backdrop-filter:bg-background/75"
                : "bg-background",
            )}
            data-testid="channel-management-sheet"
          >
            <ChannelManagementPanelContent
              activeView={activeView}
              archiveChannelMutation={archiveChannelMutation}
              canEditNarrative={canEditNarrative}
              canJoin={canJoin}
              canLeave={canLeave}
              canManageChannel={canManageChannel}
              canOpenCanvas={canOpenCanvas}
              canvasPreview={canvasPreview}
              canvasQuery={canvasQuery}
              channelId={channelId}
              deleteChannelMutation={deleteChannelMutation}
              detailsError={detailsQuery.error}
              handleDeleteChannel={handleDeleteChannel}
              handleDeleteDialogOpenChange={handleDeleteDialogOpenChange}
              isArchived={isArchived}
              isDark={isDark}
              isDeleteDialogOpen={isDeleteDialogOpen}
              canDeleteChannel={canDeleteChannel}
              mode={auxiliaryPanelMode}
              transparentChrome={transparentChrome}
              joinChannelMutation={joinChannelMutation}
              leaveChannelMutation={leaveChannelMutation}
              memberCount={memberCount}
              membersError={membersQuery.error}
              onOpenChange={handlePanelOpenChange}
              resolvedChannel={resolvedChannel}
              setActiveView={setActiveView}
              setIsEditDialogOpen={setIsEditDialogOpen}
              unarchiveChannelMutation={unarchiveChannelMutation}
            />
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      )}

      {canManageChannel ? (
        <Dialog onOpenChange={setIsEditDialogOpen} open={isEditDialogOpen}>
          <DialogContent className="max-w-lg overflow-hidden p-0">
            <div className="flex max-h-[85vh] flex-col">
              <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
                <DialogTitle>Edit channel</DialogTitle>
                <DialogDescription>
                  Update settings for{" "}
                  <span className="font-medium">{resolvedChannel.name}</span>.
                </DialogDescription>
              </DialogHeader>

              <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-5">
                <div className="space-y-3">
                  <div className="space-y-1.5">
                    <label
                      className="text-sm font-medium"
                      htmlFor="channel-name"
                    >
                      Name
                    </label>
                    <Input
                      data-testid="channel-management-name"
                      disabled={isSavingChannelEdits}
                      id="channel-name"
                      onChange={(event) => setNameDraft(event.target.value)}
                      value={nameDraft}
                    />
                  </div>
                  <div className="space-y-1.5">
                    <label
                      className="text-sm font-medium"
                      htmlFor="channel-description"
                    >
                      Description
                    </label>
                    <Textarea
                      className="min-h-24"
                      data-testid="channel-management-description"
                      disabled={isSavingChannelEdits}
                      id="channel-description"
                      onChange={(event) =>
                        setDescriptionDraft(event.target.value)
                      }
                      value={descriptionDraft}
                    />
                  </div>
                </div>

                {resolvedChannel.channelType !== "dm" ? (
                  <div
                    className="space-y-3"
                    data-testid="channel-management-lifecycle"
                  >
                    <FieldGroup>
                      <ToggleRow
                        checked={isPrivateDraft}
                        description="Only members can find and join this channel."
                        disabled={isSavingChannelEdits}
                        label="Private"
                        onCheckedChange={setIsPrivateDraft}
                        testId="channel-management-private-toggle"
                      />
                      <ToggleRow
                        checked={isEphemeralDraft}
                        description="Automatically delete this channel after a set time."
                        disabled={isSavingChannelEdits}
                        label="Ephemeral"
                        onCheckedChange={setIsEphemeralDraft}
                        testId="channel-management-ephemeral-toggle"
                      />
                    </FieldGroup>

                    {isEphemeralDraft ? (
                      <div className="space-y-1.5">
                        <label
                          className="text-sm font-medium"
                          htmlFor="channel-ttl"
                        >
                          Timeout
                        </label>
                        <Input
                          aria-invalid={ttlInvalid}
                          data-testid="channel-management-ttl"
                          disabled={isSavingChannelEdits}
                          id="channel-ttl"
                          onChange={(event) => setTtlDraft(event.target.value)}
                          placeholder="e.g. 1d, 12h, 30m"
                          value={ttlDraft}
                        />
                        <p
                          className={cn(
                            "text-xs",
                            ttlInvalid
                              ? "text-destructive"
                              : "text-muted-foreground",
                          )}
                        >
                          {ttlInvalid
                            ? "Enter a duration like 1d, 12h, or 30m."
                            : "Defaults to 7d when left empty. Resets the deletion countdown from now whenever changed."}
                        </p>
                      </div>
                    ) : null}
                  </div>
                ) : null}

                {canEditNarrative ? (
                  <div className="space-y-5">
                    <div className="space-y-3">
                      <div className="space-y-1.5">
                        <label
                          className="text-sm font-medium"
                          htmlFor="channel-topic"
                        >
                          Topic
                        </label>
                        <Input
                          data-testid="channel-management-topic"
                          disabled={isSavingChannelEdits}
                          id="channel-topic"
                          onChange={(event) =>
                            setTopicDraft(event.target.value)
                          }
                          value={topicDraft}
                        />
                      </div>
                    </div>

                    <div className="space-y-3">
                      <div className="space-y-1.5">
                        <label
                          className="text-sm font-medium"
                          htmlFor="channel-purpose"
                        >
                          Purpose
                        </label>
                        <Input
                          data-testid="channel-management-purpose"
                          disabled={isSavingChannelEdits}
                          id="channel-purpose"
                          onChange={(event) =>
                            setPurposeDraft(event.target.value)
                          }
                          value={purposeDraft}
                        />
                      </div>
                    </div>
                  </div>
                ) : null}

                {updateChannelDetailsMutation.error instanceof Error ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {updateChannelDetailsMutation.error.message}
                  </p>
                ) : null}
                {setTopicMutation.error instanceof Error ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {setTopicMutation.error.message}
                  </p>
                ) : null}
                {setPurposeMutation.error instanceof Error ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {setPurposeMutation.error.message}
                  </p>
                ) : null}
              </div>

              <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
                <Button
                  onClick={() => setIsEditDialogOpen(false)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
                <Button
                  data-testid="channel-management-save-changes"
                  disabled={!canSaveChannelEdits}
                  onClick={() => void handleSaveChannelEdits()}
                  size="sm"
                  type="button"
                >
                  {isSavingChannelEdits ? "Saving..." : "Save changes"}
                </Button>
              </div>
            </div>
          </DialogContent>
        </Dialog>
      ) : null}
    </DialogPrimitive.Root>
  );
}

type ChannelMutation<TArgs = void> = {
  error: unknown;
  isPending: boolean;
  mutateAsync: (args: TArgs) => Promise<unknown>;
};

type ChannelManagementPanelContentProps = {
  activeView: "summary" | "canvas";
  archiveChannelMutation: ChannelMutation;
  canEditNarrative: boolean;
  canJoin: boolean;
  canLeave: boolean;
  canManageChannel: boolean;
  canOpenCanvas: boolean;
  canvasPreview?: string;
  canvasQuery: { isLoading: boolean };
  channelId: string | null;
  deleteChannelMutation: ChannelMutation;
  detailsError: unknown;
  handleDeleteChannel: () => Promise<void>;
  handleDeleteDialogOpenChange: (open: boolean) => void;
  isArchived: boolean;
  isDark: boolean;
  isDeleteDialogOpen: boolean;
  canDeleteChannel: boolean; // true when caller may delete the channel
  mode: AuxiliaryPanelMode;
  transparentChrome?: boolean;
  joinChannelMutation: ChannelMutation;
  leaveChannelMutation: ChannelMutation;
  memberCount: number;
  membersError: unknown;
  onOpenChange: (open: boolean) => void;
  resolvedChannel: Channel;
  setActiveView: React.Dispatch<React.SetStateAction<"summary" | "canvas">>;
  setIsEditDialogOpen: React.Dispatch<React.SetStateAction<boolean>>;
  unarchiveChannelMutation: ChannelMutation;
};

function ChannelManagementPanelContent({
  activeView,
  archiveChannelMutation,
  canEditNarrative,
  canJoin,
  canLeave,
  canManageChannel,
  canOpenCanvas,
  canvasPreview,
  canvasQuery,
  channelId,
  deleteChannelMutation,
  detailsError,
  handleDeleteChannel,
  handleDeleteDialogOpenChange,
  isArchived,
  isDark,
  isDeleteDialogOpen,
  canDeleteChannel,
  mode,
  transparentChrome = false,
  joinChannelMutation,
  leaveChannelMutation,
  memberCount,
  membersError,
  onOpenChange,
  resolvedChannel,
  setActiveView,
  setIsEditDialogOpen,
  unarchiveChannelMutation,
}: ChannelManagementPanelContentProps) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  useScrollBoundaryLock(scrollRef);

  const showModerationActions =
    activeView === "summary" &&
    canManageChannel &&
    resolvedChannel.channelType !== "dm";
  return (
    <AuxiliaryPanelContext.Provider
      value={{
        isFloatingOverlay: mode === "panel",
        isOverlay: mode !== "docked",
        isSinglePanelView: mode === "single-panel",
        isSplitLayout: mode === "docked",
        layout: mode === "docked" ? "split" : "standalone",
        mode,
        onClose: () => onOpenChange(false),
        transparentChrome,
        widthPx: 380,
      }}
    >
      <AuxiliaryPanelHeader
        bordered={mode === "panel"}
        density={mode === "panel" ? "compact" : "comfortable"}
        mode={mode}
        transparent={transparentChrome}
      >
        <AuxiliaryPanelHeaderGroup
          backButtonAriaLabel="Back to channel"
          backButtonTestId="channel-management-back"
          mode={mode}
          onBack={
            activeView === "canvas" ? () => setActiveView("summary") : undefined
          }
        >
          <DialogPrimitive.Title asChild>
            <AuxiliaryPanelTitle>
              {activeView === "canvas" ? "Canvas" : "Channel"}
            </AuxiliaryPanelTitle>
          </DialogPrimitive.Title>
        </AuxiliaryPanelHeaderGroup>
        <DialogPrimitive.Description className="sr-only">
          Channel settings
        </DialogPrimitive.Description>
      </AuxiliaryPanelHeader>

      <AuxiliaryPanelBody
        className={cn(
          "overflow-y-auto overflow-x-hidden overscroll-contain bg-background px-4 [overflow-anchor:none]",
          showModerationActions ? "pb-20" : "pb-8",
        )}
        mode={mode}
        panelPadding
        ref={scrollRef}
      >
        {activeView === "summary" ? (
          <div className="space-y-6 pt-3">
            <ChannelHero channel={resolvedChannel} />

            {detailsError instanceof Error ? (
              <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {detailsError.message}
              </p>
            ) : null}

            {membersError instanceof Error ? (
              <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {membersError.message}
              </p>
            ) : null}

            <div className="flex flex-wrap items-start justify-center gap-6">
              <ChannelQuickAction
                icon={Copy}
                label="Copy ID"
                onClick={() => {
                  void writeTextToClipboard(resolvedChannel.id).then(() =>
                    toast.success("Copied channel ID"),
                  );
                }}
                testId="channel-management-copy-id-action"
              />
              {canJoin ? (
                <ChannelQuickAction
                  active
                  disabled={joinChannelMutation.isPending}
                  icon={DoorOpen}
                  label={joinChannelMutation.isPending ? "Joining..." : "Join"}
                  onClick={() => {
                    void joinChannelMutation.mutateAsync();
                  }}
                  testId="channel-management-join"
                />
              ) : null}
              {canLeave ? (
                <ChannelQuickAction
                  disabled={leaveChannelMutation.isPending}
                  icon={DoorClosed}
                  label={
                    leaveChannelMutation.isPending ? "Leaving..." : "Leave"
                  }
                  onClick={() => {
                    void leaveChannelMutation.mutateAsync().then(() => {
                      onOpenChange(false);
                    });
                  }}
                  testId="channel-management-leave"
                />
              ) : null}
              {canManageChannel ? (
                <ChannelQuickAction
                  icon={Pencil}
                  label="Edit"
                  onClick={() => setIsEditDialogOpen(true)}
                  testId="channel-management-edit"
                />
              ) : null}
            </div>

            {joinChannelMutation.error instanceof Error ? (
              <p className="text-center text-sm text-destructive">
                {joinChannelMutation.error.message}
              </p>
            ) : null}
            {leaveChannelMutation.error instanceof Error ? (
              <p className="text-center text-sm text-destructive">
                {leaveChannelMutation.error.message}
              </p>
            ) : null}

            {resolvedChannel.description.trim() ||
            resolvedChannel.topic?.trim() ||
            resolvedChannel.purpose?.trim() ? (
              <NarrativeGroup>
                {resolvedChannel.description.trim() ? (
                  <NarrativeField
                    icon={FileText}
                    label="Description"
                    testId="channel-management-description"
                    value={resolvedChannel.description.trim()}
                  />
                ) : null}
                {resolvedChannel.topic?.trim() ? (
                  <NarrativeField
                    icon={MessageSquare}
                    label="Topic"
                    testId="channel-management-topic"
                    value={resolvedChannel.topic.trim()}
                  />
                ) : null}
                {resolvedChannel.purpose?.trim() ? (
                  <NarrativeField
                    icon={Zap}
                    label="Purpose"
                    testId="channel-management-purpose"
                    value={resolvedChannel.purpose.trim()}
                  />
                ) : null}
              </NarrativeGroup>
            ) : null}

            {canOpenCanvas ? (
              <IngressRow
                description={canvasPreview}
                icon={BookOpenText}
                label="Canvas"
                onClick={() => setActiveView("canvas")}
                testId="channel-canvas-ingress"
                trailing={canvasQuery.isLoading ? "Loading..." : undefined}
              />
            ) : null}

            <FieldGroup>
              <CopyFieldRow
                icon={Fingerprint}
                label="Channel ID"
                testId="channel-management-channel-id"
                value={resolvedChannel.id}
              />
              <InfoFieldRow
                icon={Type}
                label="Name"
                testId="channel-management-name-row"
                value={resolvedChannel.name}
              />
              <InfoFieldRow
                icon={Radio}
                label="Type"
                testId="channel-management-type"
                value={resolvedChannel.channelType}
              />
              <InfoFieldRow
                icon={resolvedChannel.visibility === "private" ? Lock : Eye}
                label="Visibility"
                testId="channel-management-visibility"
                value={resolvedChannel.visibility}
              />
              <InfoFieldRow
                icon={Users}
                label="Members"
                testId="channel-management-member-count"
                value={`${memberCount}`}
              />
              {isArchived ? (
                <InfoFieldRow
                  icon={Archive}
                  label="Status"
                  testId="channel-management-archived"
                  value="Archived"
                />
              ) : null}
              {resolvedChannel.ttlSeconds !== null ? (
                <InfoFieldRow
                  icon={Archive}
                  label="Ephemeral"
                  testId="channel-management-ephemeral-row"
                  value={formatTtlDuration(resolvedChannel.ttlSeconds)}
                />
              ) : null}
            </FieldGroup>

            {archiveChannelMutation.error instanceof Error ? (
              <p className="text-sm text-destructive">
                {archiveChannelMutation.error.message}
              </p>
            ) : null}
            {unarchiveChannelMutation.error instanceof Error ? (
              <p className="text-sm text-destructive">
                {unarchiveChannelMutation.error.message}
              </p>
            ) : null}
          </div>
        ) : (
          <div data-testid="channel-canvas-section">
            <ChannelCanvas
              canEdit={canEditNarrative}
              channelId={channelId}
              isArchived={isArchived}
            />
          </div>
        )}
      </AuxiliaryPanelBody>

      {showModerationActions ? (
        <ChannelManagementModerationActions
          archiveChannelMutation={archiveChannelMutation}
          canManageChannel={canManageChannel}
          deleteChannelMutation={deleteChannelMutation}
          handleDeleteChannel={handleDeleteChannel}
          handleDeleteDialogOpenChange={handleDeleteDialogOpenChange}
          isArchived={isArchived}
          isDark={isDark}
          isDeleteDialogOpen={isDeleteDialogOpen}
          canDeleteChannel={canDeleteChannel}
          resolvedChannelName={resolvedChannel.name}
          unarchiveChannelMutation={unarchiveChannelMutation}
        />
      ) : null}
    </AuxiliaryPanelContext.Provider>
  );
}
