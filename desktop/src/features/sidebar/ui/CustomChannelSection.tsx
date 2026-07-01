import {
  ArrowDown,
  ArrowUp,
  Bell,
  BellOff,
  Check,
  CheckCheck,
  CheckCircle2,
  ChevronDown,
  CircleDot,
  Copy,
  EllipsisVertical,
  LogOut,
  Pencil,
  Plus,
  Star,
  StarOff,
  Trash2,
} from "lucide-react";
import { useRef, type ReactNode } from "react";

import { toast } from "sonner";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { ChannelMenuButton } from "@/features/sidebar/ui/SidebarSection";
import {
  DraggableChannelRow,
  DroppableSectionBody,
  DroppableUngroupedBody,
  SortableSectionShell,
} from "@/features/sidebar/ui/SidebarDnd";
import {
  SECTION_ACTION_VISIBILITY_CLASS,
  SECTION_ICON_BUTTON_CLASS,
} from "@/features/sidebar/ui/sidebarSectionStyles";
import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { HashSearch } from "@/shared/ui/icons";

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const CUSTOM_SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-2 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "relative size-2.5 shrink-0 text-current opacity-0 transition-[color,opacity] group-hover/sidebar-section:opacity-100 group-hover/section-label:opacity-100 group-focus-within/sidebar-section:opacity-100 group-focus-visible/section-label:opacity-100";
const SECTION_LABEL_CHEVRON_ICON_CLASS =
  "absolute left-1/2 top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2";
const CUSTOM_SECTION_ACTION_VISIBILITY_CLASS =
  "opacity-0 transition-opacity group-hover/sidebar-section:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100";
const SIDEBAR_CONTEXT_ICON_SLOT_CLASS =
  "flex h-4 w-4 shrink-0 items-center justify-center";

function deferContextMenuAction(action: () => void) {
  globalThis.setTimeout(action, 0);
}

function ContextMenuIconSlot({ children }: { children?: ReactNode }) {
  return (
    <span
      aria-hidden="true"
      className={SIDEBAR_CONTEXT_ICON_SLOT_CLASS}
      data-sidebar-context-icon-slot
    >
      {children}
    </span>
  );
}

function MoveToSectionSubmenu({
  channelId,
  sections,
  assignments,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
}: {
  channelId: string;
  sections: ChannelSection[];
  assignments: Record<string, string>;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onUnassignChannel: (channelId: string) => void;
  onCreateSectionForChannel: (channelId: string) => void;
}) {
  const currentSectionId = assignments[channelId];

  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <ContextMenuIconSlot />
        <span>Move to section</span>
      </ContextMenuSubTrigger>
      <ContextMenuSubContent>
        {sections.map((section) => (
          <ContextMenuItem
            key={section.id}
            onSelect={() =>
              deferContextMenuAction(() =>
                onAssignChannel(channelId, section.id),
              )
            }
          >
            <ContextMenuIconSlot>
              {currentSectionId === section.id ? (
                <Check className="h-4 w-4" />
              ) : section.icon ? (
                <StatusEmoji className="h-4 w-4" value={section.icon} />
              ) : null}
            </ContextMenuIconSlot>
            <span>{section.name}</span>
          </ContextMenuItem>
        ))}
        {sections.length > 0 ? <ContextMenuSeparator /> : null}
        <ContextMenuItem
          onSelect={() =>
            deferContextMenuAction(() => onCreateSectionForChannel(channelId))
          }
        >
          <ContextMenuIconSlot>
            <Plus className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>New section...</span>
        </ContextMenuItem>
        {currentSectionId ? (
          <ContextMenuItem
            onSelect={() =>
              deferContextMenuAction(() => onUnassignChannel(channelId))
            }
          >
            <ContextMenuIconSlot />
            <span>Remove from section</span>
          </ContextMenuItem>
        ) : null}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

function copyToClipboard(text: string, successMessage: string) {
  void navigator.clipboard
    .writeText(text)
    .then(() => {
      toast.success(successMessage);
    })
    .catch(() => {
      toast.error("Failed to copy to clipboard");
    });
}

function CopyChannelSubmenu({ channel }: { channel: Channel }) {
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <ContextMenuIconSlot>
          <Copy className="h-4 w-4" />
        </ContextMenuIconSlot>
        <span>Copy</span>
      </ContextMenuSubTrigger>
      <ContextMenuSubContent>
        <ContextMenuItem
          onSelect={() =>
            copyToClipboard(channel.name, "Channel name copied to clipboard")
          }
        >
          <span>Copy channel name</span>
        </ContextMenuItem>
        <ContextMenuItem
          onSelect={() =>
            copyToClipboard(channel.id, "Channel ID copied to clipboard")
          }
        >
          <span>Copy channel ID</span>
        </ContextMenuItem>
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

export function ChannelContextMenuItems({
  channel,
  hasUnread,
  isMuted,
  isStarred,
  sections,
  assignments,
  onMarkChannelRead,
  onMarkChannelUnread,
  onMuteChannel,
  onUnmuteChannel,
  onStarChannel,
  onUnstarChannel,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
  onLeaveChannel,
}: {
  channel: Channel;
  hasUnread: boolean;
  isMuted?: boolean;
  isStarred?: boolean;
  sections?: ChannelSection[];
  assignments?: Record<string, string>;
  onMarkChannelRead?: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread?: (channelId: string) => void;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
  onAssignChannel?: (channelId: string, sectionId: string) => void;
  onUnassignChannel?: (channelId: string) => void;
  onCreateSectionForChannel?: (channelId: string) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const showStar = Boolean(onStarChannel && onUnstarChannel);
  const showReadToggle = hasUnread
    ? Boolean(onMarkChannelRead)
    : Boolean(onMarkChannelUnread);
  const showMuteToggle = Boolean(onMuteChannel && onUnmuteChannel);
  const showMove = Boolean(
    sections &&
      assignments &&
      onAssignChannel &&
      onUnassignChannel &&
      onCreateSectionForChannel,
  );

  return (
    <>
      <CopyChannelSubmenu channel={channel} />
      {showMove ? (
        <MoveToSectionSubmenu
          channelId={channel.id}
          sections={sections ?? []}
          assignments={assignments ?? {}}
          onAssignChannel={onAssignChannel ?? (() => {})}
          onUnassignChannel={onUnassignChannel ?? (() => {})}
          onCreateSectionForChannel={onCreateSectionForChannel ?? (() => {})}
        />
      ) : null}
      {showReadToggle ? <ContextMenuSeparator /> : null}
      {hasUnread && onMarkChannelRead ? (
        <ContextMenuItem
          onSelect={() =>
            deferContextMenuAction(() =>
              onMarkChannelRead(channel.id, channel.lastMessageAt),
            )
          }
        >
          <ContextMenuIconSlot>
            <CheckCircle2 className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Mark as read</span>
        </ContextMenuItem>
      ) : !hasUnread && onMarkChannelUnread ? (
        <ContextMenuItem
          onSelect={() =>
            deferContextMenuAction(() => onMarkChannelUnread(channel.id))
          }
        >
          <ContextMenuIconSlot>
            <CircleDot className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Mark unread</span>
        </ContextMenuItem>
      ) : null}
      {showMuteToggle || showStar ? <ContextMenuSeparator /> : null}
      {showMuteToggle ? (
        isMuted ? (
          <ContextMenuItem
            onSelect={() =>
              deferContextMenuAction(() => onUnmuteChannel?.(channel.id))
            }
          >
            <ContextMenuIconSlot>
              <Bell className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Unmute channel</span>
          </ContextMenuItem>
        ) : (
          <ContextMenuItem
            onSelect={() =>
              deferContextMenuAction(() => onMuteChannel?.(channel.id))
            }
          >
            <ContextMenuIconSlot>
              <BellOff className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Mute channel</span>
          </ContextMenuItem>
        )
      ) : null}
      {showStar ? (
        isStarred ? (
          <ContextMenuItem
            onSelect={() =>
              deferContextMenuAction(() => onUnstarChannel?.(channel.id))
            }
          >
            <ContextMenuIconSlot>
              <StarOff className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Unstar channel</span>
          </ContextMenuItem>
        ) : (
          <ContextMenuItem
            onSelect={() =>
              deferContextMenuAction(() => onStarChannel?.(channel.id))
            }
          >
            <ContextMenuIconSlot>
              <Star className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Star channel</span>
          </ContextMenuItem>
        )
      ) : null}
      {onLeaveChannel ? (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem
            className="text-destructive focus:text-destructive"
            onSelect={() =>
              deferContextMenuAction(() => onLeaveChannel(channel))
            }
          >
            <ContextMenuIconSlot>
              <LogOut className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Leave channel</span>
          </ContextMenuItem>
        </>
      ) : null}
    </>
  );
}

function SectionHeaderActions({
  browseAriaLabel,
  createAriaLabel,
  hasUnread,
  onBrowseClick,
  onCreateClick,
  onMarkAllRead,
}: {
  browseAriaLabel?: string;
  createAriaLabel: string;
  hasUnread?: boolean;
  onBrowseClick?: () => void;
  onCreateClick?: () => void;
  onMarkAllRead?: () => void;
}) {
  return (
    <div className="absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5">
      {hasUnread && onMarkAllRead ? (
        <button
          aria-label="Mark all as read"
          className={cn(
            SECTION_ICON_BUTTON_CLASS,
            SECTION_ACTION_VISIBILITY_CLASS,
          )}
          onClick={onMarkAllRead}
          title="Mark all as read"
          type="button"
        >
          <CheckCheck className="h-4 w-4" />
        </button>
      ) : null}
      {onBrowseClick ? (
        <button
          aria-label={browseAriaLabel}
          className={cn(
            SECTION_ICON_BUTTON_CLASS,
            SECTION_ACTION_VISIBILITY_CLASS,
          )}
          onClick={onBrowseClick}
          title={browseAriaLabel}
          type="button"
        >
          <HashSearch className="h-4 w-4" />
        </button>
      ) : null}
      {onCreateClick ? (
        <button
          aria-label={createAriaLabel}
          className={cn(
            SECTION_ICON_BUTTON_CLASS,
            SECTION_ACTION_VISIBILITY_CLASS,
          )}
          onClick={onCreateClick}
          type="button"
        >
          <Plus className="h-4 w-4" />
        </button>
      ) : null}
    </div>
  );
}

function CustomSectionActionsMenu({
  sectionId,
  sectionName,
  hasUnread,
  isFirst,
  isLast,
  onMarkSectionRead,
  onRenameSection,
  onDeleteSection,
  onMoveSectionUp,
  onMoveSectionDown,
}: {
  sectionId: string;
  sectionName: string;
  hasUnread: boolean;
  isFirst: boolean;
  isLast: boolean;
  onMarkSectionRead: () => void;
  onRenameSection: () => void;
  onDeleteSection: () => void;
  onMoveSectionUp: () => void;
  onMoveSectionDown: () => void;
}) {
  const triggerRef = useRef<HTMLButtonElement>(null);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Open actions for ${sectionName}`}
          className={cn(
            SECTION_ICON_BUTTON_CLASS,
            CUSTOM_SECTION_ACTION_VISIBILITY_CLASS,
          )}
          data-testid={`section-actions-${sectionId}`}
          onClick={(event) => event.stopPropagation()}
          onPointerDown={(event) => event.stopPropagation()}
          ref={triggerRef}
          type="button"
        >
          <EllipsisVertical className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          triggerRef.current?.blur();
        }}
      >
        {hasUnread ? (
          <DropdownMenuItem
            onSelect={() => deferContextMenuAction(onMarkSectionRead)}
          >
            <CheckCheck className="h-4 w-4" />
            <span>Mark all as read</span>
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem
          onSelect={() => deferContextMenuAction(onRenameSection)}
        >
          <Pencil className="h-4 w-4" />
          <span>Rename section</span>
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={isFirst}
          onSelect={() => deferContextMenuAction(onMoveSectionUp)}
        >
          <ArrowUp className="h-4 w-4" />
          <span>Move up</span>
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={isLast}
          onSelect={() => deferContextMenuAction(onMoveSectionDown)}
        >
          <ArrowDown className="h-4 w-4" />
          <span>Move down</span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          onSelect={() => deferContextMenuAction(onDeleteSection)}
        >
          <Trash2 className="h-4 w-4" />
          <span>Delete section</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ChannelGroupSection({
  browseAriaLabel,
  createAriaLabel,
  draggable,
  groupClassName,
  hasUnread,
  isCollapsed,
  isActiveChannel,
  activeWorkingByChannelId,
  items,
  listTestId,
  onBrowseClick,
  onCreateClick,
  onMarkAllRead,
  onMarkChannelRead,
  onMarkChannelUnread,
  onSelectChannel,
  onToggleCollapsed,
  selectedChannelId,
  title,
  unreadChannelCounts,
  unreadChannelIds,
  sections,
  assignments,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
  mutedChannelIds,
  onMuteChannel,
  onUnmuteChannel,
  starredChannelIds,
  onStarChannel,
  onUnstarChannel,
  onLeaveChannel,
}: {
  browseAriaLabel?: string;
  createAriaLabel: string;
  draggable?: boolean;
  groupClassName?: string;
  isCollapsed: boolean;
  isActiveChannel: boolean;
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  items: Channel[];
  listTestId: string;
  onBrowseClick?: () => void;
  onCreateClick?: () => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onToggleCollapsed: () => void;
  selectedChannelId: string | null;
  title: string;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  hasUnread?: boolean;
  onMarkAllRead?: () => void;
  sections?: ChannelSection[];
  assignments?: Record<string, string>;
  onAssignChannel?: (channelId: string, sectionId: string) => void;
  onUnassignChannel?: (channelId: string) => void;
  onCreateSectionForChannel?: (channelId: string) => void;
  mutedChannelIds?: ReadonlySet<string>;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  starredChannelIds?: ReadonlySet<string>;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const contentId = `sidebar-${listTestId}`;

  const channelList =
    items.length > 0 ? (
      <SidebarMenu data-testid={listTestId}>
        {items.map((channel) => (
          <ContextMenu key={channel.id}>
            <ContextMenuTrigger asChild>
              <SidebarMenuItem className="content-visibility-auto-row">
                {draggable ? (
                  <DraggableChannelRow channelId={channel.id}>
                    <ChannelMenuButton
                      channel={channel}
                      activeWorking={activeWorkingByChannelId?.get(channel.id)}
                      hasUnread={unreadChannelIds.has(channel.id)}
                      unreadCount={unreadChannelCounts.get(channel.id) ?? 0}
                      isMuted={mutedChannelIds?.has(channel.id)}
                      isActive={
                        isActiveChannel && selectedChannelId === channel.id
                      }
                      onSelectChannel={onSelectChannel}
                    />
                  </DraggableChannelRow>
                ) : (
                  <ChannelMenuButton
                    channel={channel}
                    activeWorking={activeWorkingByChannelId?.get(channel.id)}
                    hasUnread={unreadChannelIds.has(channel.id)}
                    unreadCount={unreadChannelCounts.get(channel.id) ?? 0}
                    isMuted={mutedChannelIds?.has(channel.id)}
                    isActive={
                      isActiveChannel && selectedChannelId === channel.id
                    }
                    onSelectChannel={onSelectChannel}
                  />
                )}
              </SidebarMenuItem>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ChannelContextMenuItems
                channel={channel}
                hasUnread={unreadChannelIds.has(channel.id)}
                isMuted={mutedChannelIds?.has(channel.id)}
                isStarred={starredChannelIds?.has(channel.id)}
                sections={sections}
                assignments={assignments}
                onMarkChannelRead={onMarkChannelRead}
                onMarkChannelUnread={onMarkChannelUnread}
                onMuteChannel={onMuteChannel}
                onUnmuteChannel={onUnmuteChannel}
                onStarChannel={onStarChannel}
                onUnstarChannel={onUnstarChannel}
                onAssignChannel={onAssignChannel}
                onUnassignChannel={onUnassignChannel}
                onCreateSectionForChannel={onCreateSectionForChannel}
                onLeaveChannel={onLeaveChannel}
              />
            </ContextMenuContent>
          </ContextMenu>
        ))}
      </SidebarMenu>
    ) : null;

  const sectionContent = (
    <SidebarGroup
      className={cn("group/sidebar-section select-none", groupClassName)}
    >
      <div className="relative">
        <SidebarGroupLabel asChild>
          <button
            aria-controls={contentId}
            aria-expanded={!isCollapsed}
            className={SECTION_LABEL_BUTTON_CLASS}
            onClick={onToggleCollapsed}
            type="button"
          >
            <span>{title}</span>
            <span aria-hidden="true" className={SECTION_LABEL_CHEVRON_CLASS}>
              <ChevronDown
                className={cn(
                  SECTION_LABEL_CHEVRON_ICON_CLASS,
                  isCollapsed ? "-rotate-90" : "rotate-0",
                )}
              />
            </span>
          </button>
        </SidebarGroupLabel>
        <SectionHeaderActions
          browseAriaLabel={browseAriaLabel}
          createAriaLabel={createAriaLabel}
          hasUnread={hasUnread}
          onBrowseClick={onBrowseClick}
          onCreateClick={onCreateClick}
          onMarkAllRead={onMarkAllRead}
        />
      </div>
      {!isCollapsed ? (
        <SidebarGroupContent id={contentId}>{channelList}</SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );

  return draggable ? (
    <DroppableUngroupedBody>{sectionContent}</DroppableUngroupedBody>
  ) : (
    sectionContent
  );
}

export function CustomChannelSection({
  section,
  channels,
  hasUnread,
  isCollapsed,
  isActiveChannel,
  activeWorkingByChannelId,
  selectedChannelId,
  unreadChannelCounts,
  unreadChannelIds,
  sections,
  assignments,
  isFirst,
  isLast,
  onToggleCollapsed,
  onSelectChannel,
  onMarkChannelRead,
  onMarkChannelUnread,
  onMarkSectionRead,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
  onRenameSection,
  onDeleteSection,
  onMoveSectionUp,
  onMoveSectionDown,
  mutedChannelIds,
  onMuteChannel,
  onUnmuteChannel,
  starredChannelIds,
  onStarChannel,
  onUnstarChannel,
  onLeaveChannel,
}: {
  section: ChannelSection;
  channels: Channel[];
  hasUnread: boolean;
  isCollapsed: boolean;
  isActiveChannel: boolean;
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  selectedChannelId: string | null;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  sections: ChannelSection[];
  assignments: Record<string, string>;
  isFirst: boolean;
  isLast: boolean;
  onToggleCollapsed: () => void;
  onSelectChannel: (channelId: string) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onMarkSectionRead: () => void;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onUnassignChannel: (channelId: string) => void;
  onCreateSectionForChannel: (channelId: string) => void;
  onRenameSection: () => void;
  onDeleteSection: () => void;
  onMoveSectionUp: () => void;
  onMoveSectionDown: () => void;
  mutedChannelIds?: ReadonlySet<string>;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  starredChannelIds?: ReadonlySet<string>;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const contentId = `sidebar-section-${section.id}`;

  return (
    <SortableSectionShell sectionId={section.id}>
      {({ dragHandleProps, isDragging }) => (
        <DroppableSectionBody sectionId={section.id}>
          <SidebarGroup
            className={cn(
              "group/sidebar-section select-none",
              isDragging && "opacity-30",
            )}
          >
            <ContextMenu>
              <ContextMenuTrigger asChild>
                <div
                  className="group/section-header relative"
                  {...dragHandleProps}
                >
                  <SidebarGroupLabel asChild>
                    <button
                      aria-controls={contentId}
                      aria-expanded={!isCollapsed}
                      className={CUSTOM_SECTION_LABEL_BUTTON_CLASS}
                      onClick={onToggleCollapsed}
                      type="button"
                    >
                      {section.icon ? (
                        <span
                          aria-hidden="true"
                          className="flex h-4 w-4 shrink-0 items-center justify-center"
                          data-testid={`section-icon-${section.id}`}
                        >
                          <StatusEmoji
                            className="h-4 w-4"
                            value={section.icon}
                          />
                        </span>
                      ) : null}
                      <span
                        className="truncate"
                        data-testid={`section-title-${section.id}`}
                      >
                        {section.name}
                      </span>
                      <span
                        aria-hidden="true"
                        className={SECTION_LABEL_CHEVRON_CLASS}
                      >
                        <ChevronDown
                          className={cn(
                            SECTION_LABEL_CHEVRON_ICON_CLASS,
                            isCollapsed ? "-rotate-90" : "rotate-0",
                          )}
                        />
                      </span>
                    </button>
                  </SidebarGroupLabel>
                  <div className="absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5">
                    <CustomSectionActionsMenu
                      sectionId={section.id}
                      sectionName={section.name}
                      hasUnread={hasUnread}
                      isFirst={isFirst}
                      isLast={isLast}
                      onMarkSectionRead={onMarkSectionRead}
                      onRenameSection={onRenameSection}
                      onDeleteSection={onDeleteSection}
                      onMoveSectionUp={onMoveSectionUp}
                      onMoveSectionDown={onMoveSectionDown}
                    />
                  </div>
                </div>
              </ContextMenuTrigger>
              <ContextMenuContent>
                {hasUnread ? (
                  <ContextMenuItem
                    onSelect={() => deferContextMenuAction(onMarkSectionRead)}
                  >
                    <ContextMenuIconSlot>
                      <CheckCheck className="h-4 w-4" />
                    </ContextMenuIconSlot>
                    <span>Mark all as read</span>
                  </ContextMenuItem>
                ) : null}
                <ContextMenuItem
                  onSelect={() => deferContextMenuAction(onRenameSection)}
                >
                  <ContextMenuIconSlot>
                    <Pencil className="h-4 w-4" />
                  </ContextMenuIconSlot>
                  <span>Rename section</span>
                </ContextMenuItem>
                <ContextMenuItem
                  disabled={isFirst}
                  onSelect={() => deferContextMenuAction(onMoveSectionUp)}
                >
                  <ContextMenuIconSlot>
                    <ArrowUp className="h-4 w-4" />
                  </ContextMenuIconSlot>
                  <span>Move up</span>
                </ContextMenuItem>
                <ContextMenuItem
                  disabled={isLast}
                  onSelect={() => deferContextMenuAction(onMoveSectionDown)}
                >
                  <ContextMenuIconSlot>
                    <ArrowDown className="h-4 w-4" />
                  </ContextMenuIconSlot>
                  <span>Move down</span>
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem
                  className="text-destructive focus:text-destructive"
                  onSelect={() => deferContextMenuAction(onDeleteSection)}
                >
                  <ContextMenuIconSlot>
                    <Trash2 className="h-4 w-4" />
                  </ContextMenuIconSlot>
                  <span>Delete section</span>
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
            {!isCollapsed ? (
              <SidebarGroupContent id={contentId}>
                {channels.length > 0 ? (
                  <SidebarMenu>
                    {channels.map((channel) => (
                      <ContextMenu key={channel.id}>
                        <ContextMenuTrigger asChild>
                          <SidebarMenuItem>
                            <DraggableChannelRow channelId={channel.id}>
                              <ChannelMenuButton
                                channel={channel}
                                activeWorking={activeWorkingByChannelId?.get(
                                  channel.id,
                                )}
                                hasUnread={unreadChannelIds.has(channel.id)}
                                unreadCount={
                                  unreadChannelCounts.get(channel.id) ?? 0
                                }
                                isMuted={mutedChannelIds?.has(channel.id)}
                                isActive={
                                  isActiveChannel &&
                                  selectedChannelId === channel.id
                                }
                                onSelectChannel={onSelectChannel}
                              />
                            </DraggableChannelRow>
                          </SidebarMenuItem>
                        </ContextMenuTrigger>
                        <ContextMenuContent>
                          <ChannelContextMenuItems
                            channel={channel}
                            hasUnread={unreadChannelIds.has(channel.id)}
                            isMuted={mutedChannelIds?.has(channel.id)}
                            isStarred={starredChannelIds?.has(channel.id)}
                            sections={sections}
                            assignments={assignments}
                            onMarkChannelRead={onMarkChannelRead}
                            onMarkChannelUnread={onMarkChannelUnread}
                            onMuteChannel={onMuteChannel}
                            onUnmuteChannel={onUnmuteChannel}
                            onStarChannel={onStarChannel}
                            onUnstarChannel={onUnstarChannel}
                            onAssignChannel={onAssignChannel}
                            onUnassignChannel={onUnassignChannel}
                            onCreateSectionForChannel={
                              onCreateSectionForChannel
                            }
                            onLeaveChannel={onLeaveChannel}
                          />
                        </ContextMenuContent>
                      </ContextMenu>
                    ))}
                  </SidebarMenu>
                ) : null}
              </SidebarGroupContent>
            ) : null}
          </SidebarGroup>
        </DroppableSectionBody>
      )}
    </SortableSectionShell>
  );
}
