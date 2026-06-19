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
  GripVertical,
  Pencil,
  Plus,
  Search,
  Star,
  StarOff,
  Trash2,
} from "lucide-react";

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
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

// ---------------------------------------------------------------------------
// Shared styles
// ---------------------------------------------------------------------------

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "relative size-2.5 shrink-0 opacity-0 text-sidebar-foreground/45 transition-[color,opacity] group-hover/sidebar-section:opacity-100 group-hover/sidebar-section:text-sidebar-foreground group-hover/section-label:opacity-100 group-hover/section-label:text-sidebar-foreground group-focus-within/sidebar-section:opacity-100 group-focus-within/sidebar-section:text-sidebar-foreground group-focus-visible/section-label:opacity-100 group-focus-visible/section-label:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_ICON_CLASS =
  "absolute left-1/2 top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2";

// ---------------------------------------------------------------------------
// MoveToSectionSubmenu — internal helper
// ---------------------------------------------------------------------------

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
      <ContextMenuSubTrigger>Move to section</ContextMenuSubTrigger>
      <ContextMenuSubContent>
        {sections.map((section) => (
          <ContextMenuItem
            key={section.id}
            onClick={() => onAssignChannel(channelId, section.id)}
          >
            {currentSectionId === section.id ? (
              <Check className="h-4 w-4" />
            ) : (
              <span className="h-4 w-4" />
            )}
            {section.name}
          </ContextMenuItem>
        ))}
        {sections.length > 0 ? <ContextMenuSeparator /> : null}
        <ContextMenuItem onClick={() => onCreateSectionForChannel(channelId)}>
          <Plus className="h-4 w-4" />
          New section...
        </ContextMenuItem>
        {currentSectionId ? (
          <ContextMenuItem onClick={() => onUnassignChannel(channelId)}>
            Remove from section
          </ContextMenuItem>
        ) : null}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

// ---------------------------------------------------------------------------
// ChannelContextMenuItems — shared context menu items for channel rows
// ---------------------------------------------------------------------------

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
}) {
  const showStar = Boolean(onStarChannel && onUnstarChannel);
  const showReadToggle = hasUnread
    ? Boolean(onMarkChannelRead)
    : Boolean(onMarkChannelUnread);
  return (
    <>
      {showStar ? (
        isStarred ? (
          <ContextMenuItem onClick={() => onUnstarChannel?.(channel.id)}>
            <StarOff className="h-4 w-4" />
            Unstar channel
          </ContextMenuItem>
        ) : (
          <ContextMenuItem onClick={() => onStarChannel?.(channel.id)}>
            <Star className="h-4 w-4" />
            Star channel
          </ContextMenuItem>
        )
      ) : null}
      {showStar && showReadToggle ? <ContextMenuSeparator /> : null}
      {hasUnread && onMarkChannelRead ? (
        <ContextMenuItem
          onClick={() => onMarkChannelRead(channel.id, channel.lastMessageAt)}
        >
          <CheckCircle2 className="h-4 w-4" />
          Mark as read
        </ContextMenuItem>
      ) : !hasUnread && onMarkChannelUnread ? (
        <ContextMenuItem onClick={() => onMarkChannelUnread(channel.id)}>
          <CircleDot className="h-4 w-4" />
          Mark unread
        </ContextMenuItem>
      ) : null}
      {onMuteChannel && onUnmuteChannel ? (
        <>
          <ContextMenuSeparator />
          {isMuted ? (
            <ContextMenuItem onClick={() => onUnmuteChannel(channel.id)}>
              <Bell className="h-4 w-4" />
              Unmute channel
            </ContextMenuItem>
          ) : (
            <ContextMenuItem onClick={() => onMuteChannel(channel.id)}>
              <BellOff className="h-4 w-4" />
              Mute channel
            </ContextMenuItem>
          )}
        </>
      ) : null}
      {sections &&
      assignments &&
      onAssignChannel &&
      onUnassignChannel &&
      onCreateSectionForChannel ? (
        <>
          <ContextMenuSeparator />
          <MoveToSectionSubmenu
            channelId={channel.id}
            sections={sections}
            assignments={assignments}
            onAssignChannel={onAssignChannel}
            onUnassignChannel={onUnassignChannel}
            onCreateSectionForChannel={onCreateSectionForChannel}
          />
        </>
      ) : null}
    </>
  );
}

// ---------------------------------------------------------------------------
// SectionHeaderActions — create and read-state icon buttons for section headers
// ---------------------------------------------------------------------------

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
          className={SECTION_ICON_BUTTON_CLASS}
          onClick={onBrowseClick}
          title={browseAriaLabel}
          type="button"
        >
          <Search className="h-4 w-4" />
        </button>
      ) : null}
      {onCreateClick ? (
        <button
          aria-label={createAriaLabel}
          className={SECTION_ICON_BUTTON_CLASS}
          onClick={onCreateClick}
          type="button"
        >
          <Plus className="h-4 w-4" />
        </button>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ChannelGroupSection — unified Channels / Forums section (no inline form)
// ---------------------------------------------------------------------------

export function ChannelGroupSection({
  browseAriaLabel,
  createAriaLabel,
  draggable,
  groupClassName,
  hasUnread,
  isCollapsed,
  isActiveChannel,
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
}: {
  browseAriaLabel?: string;
  createAriaLabel: string;
  draggable?: boolean;
  groupClassName?: string;
  isCollapsed: boolean;
  isActiveChannel: boolean;
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
              />
            </ContextMenuContent>
          </ContextMenu>
        ))}
      </SidebarMenu>
    ) : null;

  const sectionContent = (
    <SidebarGroup className={groupClassName}>
      <div className="group/sidebar-section relative">
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

// ---------------------------------------------------------------------------
// CustomChannelSection — user-defined channel section with management actions
// ---------------------------------------------------------------------------

export function CustomChannelSection({
  section,
  channels,
  hasUnread,
  isCollapsed,
  isActiveChannel,
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
}: {
  section: ChannelSection;
  channels: Channel[];
  hasUnread: boolean;
  isCollapsed: boolean;
  isActiveChannel: boolean;
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
}) {
  const contentId = `sidebar-section-${section.id}`;

  return (
    <SortableSectionShell sectionId={section.id}>
      {({ dragHandleProps, isDragging }) => (
        <DroppableSectionBody sectionId={section.id}>
          <SidebarGroup className={cn(isDragging && "opacity-30")}>
            <ContextMenu>
              <ContextMenuTrigger asChild>
                <div
                  className="group/sidebar-section relative"
                  {...dragHandleProps}
                >
                  <SidebarGroupLabel asChild>
                    <button
                      aria-controls={contentId}
                      aria-expanded={!isCollapsed}
                      className={SECTION_LABEL_BUTTON_CLASS}
                      onClick={onToggleCollapsed}
                      type="button"
                    >
                      <GripVertical
                        className={cn(
                          "h-4 w-4 shrink-0 text-sidebar-foreground/30",
                          SECTION_ACTION_VISIBILITY_CLASS,
                        )}
                        aria-hidden="true"
                      />
                      <span>{section.name}</span>
                      <ChevronDown
                        aria-hidden="true"
                        className={cn(
                          SECTION_LABEL_CHEVRON_CLASS,
                          isCollapsed ? "-rotate-90" : "rotate-0",
                        )}
                      />
                    </button>
                  </SidebarGroupLabel>
                  <div
                    className={cn(
                      "absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5",
                      SECTION_ACTION_VISIBILITY_CLASS,
                    )}
                  >
                    {hasUnread ? (
                      <button
                        aria-label="Mark all as read"
                        className={SECTION_ICON_BUTTON_CLASS}
                        onClick={(e) => {
                          e.stopPropagation();
                          onMarkSectionRead();
                        }}
                        title="Mark all as read"
                        type="button"
                      >
                        <CheckCheck className="h-4 w-4" />
                      </button>
                    ) : null}
                    <button
                      aria-label="Rename section"
                      className={SECTION_ICON_BUTTON_CLASS}
                      onClick={(e) => {
                        e.stopPropagation();
                        onRenameSection();
                      }}
                      type="button"
                    >
                      <Pencil className="h-4 w-4" />
                    </button>
                    <button
                      aria-label="Delete section"
                      className={SECTION_ICON_BUTTON_CLASS}
                      onClick={(e) => {
                        e.stopPropagation();
                        onDeleteSection();
                      }}
                      type="button"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem onClick={onRenameSection}>
                  <Pencil className="h-4 w-4" />
                  Rename section
                </ContextMenuItem>
                <ContextMenuItem disabled={isFirst} onClick={onMoveSectionUp}>
                  <ArrowUp className="h-4 w-4" />
                  Move up
                </ContextMenuItem>
                <ContextMenuItem disabled={isLast} onClick={onMoveSectionDown}>
                  <ArrowDown className="h-4 w-4" />
                  Move down
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem
                  className="text-destructive focus:text-destructive"
                  onClick={onDeleteSection}
                >
                  <Trash2 className="h-4 w-4" />
                  Delete section
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
