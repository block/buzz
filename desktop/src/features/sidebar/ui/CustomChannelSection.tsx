import {
  ArrowDown,
  ArrowUp,
  Check,
  CheckCheck,
  CheckCircle2,
  ChevronDown,
  CircleDot,
  GripVertical,
  Pencil,
  Plus,
  Search,
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
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

// ---------------------------------------------------------------------------
// Shared styles
// ---------------------------------------------------------------------------

const SECTION_ICON_BUTTON_CLASS =
  "flex h-5 w-5 items-center justify-center rounded-md text-sidebar-foreground/50 hover:bg-sidebar-accent/60 hover:text-sidebar-foreground";
export const SECTION_ACTION_VISIBILITY_CLASS =
  "opacity-0 transition-opacity group-hover/sidebar-section:opacity-100 group-focus-within/sidebar-section:opacity-100";
const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "h-2.5 w-2.5 shrink-0 opacity-0 text-sidebar-foreground/45 transition-[color,opacity,transform] group-hover/section-label:opacity-100 group-hover/section-label:text-sidebar-foreground group-focus-visible/section-label:opacity-100 group-focus-visible/section-label:text-sidebar-foreground";

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
  sections,
  assignments,
  onMarkChannelRead,
  onMarkChannelUnread,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
}: {
  channel: Channel;
  hasUnread: boolean;
  sections?: ChannelSection[];
  assignments?: Record<string, string>;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onAssignChannel?: (channelId: string, sectionId: string) => void;
  onUnassignChannel?: (channelId: string) => void;
  onCreateSectionForChannel?: (channelId: string) => void;
}) {
  return (
    <>
      {hasUnread ? (
        <ContextMenuItem
          onClick={() => onMarkChannelRead(channel.id, channel.lastMessageAt)}
        >
          <CheckCircle2 className="h-4 w-4" />
          Mark as read
        </ContextMenuItem>
      ) : (
        <ContextMenuItem
          onClick={() => onMarkChannelUnread(channel.id, channel.lastMessageAt)}
        >
          <CircleDot className="h-4 w-4" />
          Mark unread
        </ContextMenuItem>
      )}
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
// SectionHeaderActions — browse + create icon buttons for section headers
// ---------------------------------------------------------------------------

function SectionHeaderActions({
  browseAriaLabel,
  browseTestId,
  className,
  createAriaLabel,
  hasUnread,
  onBrowse,
  onCreateClick,
  onMarkAllRead,
}: {
  browseAriaLabel: string;
  browseTestId?: string;
  className?: string;
  createAriaLabel: string;
  hasUnread?: boolean;
  onBrowse: () => void;
  onCreateClick: () => void;
  onMarkAllRead?: () => void;
}) {
  return (
    <div
      className={cn(
        "absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5",
        className,
      )}
    >
      {hasUnread && onMarkAllRead ? (
        <button
          aria-label="Mark all as read"
          className={SECTION_ICON_BUTTON_CLASS}
          onClick={onMarkAllRead}
          title="Mark all as read"
          type="button"
        >
          <CheckCheck className="h-3.5 w-3.5" />
        </button>
      ) : null}
      <button
        aria-label={browseAriaLabel}
        className={SECTION_ICON_BUTTON_CLASS}
        data-testid={browseTestId}
        onClick={onBrowse}
        type="button"
      >
        <Search className="h-3.5 w-3.5" />
      </button>
      <button
        aria-label={createAriaLabel}
        className={SECTION_ICON_BUTTON_CLASS}
        onClick={onCreateClick}
        type="button"
      >
        <Plus className="h-4 w-4" />
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ChannelGroupSection — unified Channels / Forums section (no inline form)
// ---------------------------------------------------------------------------

export function ChannelGroupSection({
  browseAriaLabel,
  browseTestId,
  createAriaLabel,
  draggable,
  groupClassName,
  hasUnread,
  isCollapsed,
  isActiveChannel,
  items,
  listTestId,
  onBrowse,
  onCreateClick,
  onMarkAllRead,
  onMarkChannelRead,
  onMarkChannelUnread,
  onSelectChannel,
  onToggleCollapsed,
  selectedChannelId,
  title,
  unreadChannelIds,
  sections,
  assignments,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
}: {
  browseAriaLabel: string;
  browseTestId?: string;
  createAriaLabel: string;
  draggable?: boolean;
  groupClassName?: string;
  isCollapsed: boolean;
  isActiveChannel: boolean;
  items: Channel[];
  listTestId: string;
  onBrowse: () => void;
  onCreateClick: () => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onSelectChannel: (channelId: string) => void;
  onToggleCollapsed: () => void;
  selectedChannelId: string | null;
  title: string;
  unreadChannelIds: Set<string>;
  hasUnread?: boolean;
  onMarkAllRead?: () => void;
  sections?: ChannelSection[];
  assignments?: Record<string, string>;
  onAssignChannel?: (channelId: string, sectionId: string) => void;
  onUnassignChannel?: (channelId: string) => void;
  onCreateSectionForChannel?: (channelId: string) => void;
}) {
  const contentId = `sidebar-${listTestId}`;

  const channelList =
    items.length > 0 ? (
      <SidebarMenu data-testid={listTestId}>
        {items.map((channel) => (
          <ContextMenu key={channel.id}>
            <ContextMenuTrigger asChild>
              <SidebarMenuItem>
                {draggable ? (
                  <DraggableChannelRow channelId={channel.id}>
                    <ChannelMenuButton
                      channel={channel}
                      hasUnread={unreadChannelIds.has(channel.id)}
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
                sections={sections}
                assignments={assignments}
                onMarkChannelRead={onMarkChannelRead}
                onMarkChannelUnread={onMarkChannelUnread}
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
            <ChevronDown
              aria-hidden="true"
              className={cn(
                SECTION_LABEL_CHEVRON_CLASS,
                isCollapsed ? "-rotate-90" : "rotate-0",
              )}
            />
          </button>
        </SidebarGroupLabel>
        <SectionHeaderActions
          browseAriaLabel={browseAriaLabel}
          browseTestId={browseTestId}
          className={SECTION_ACTION_VISIBILITY_CLASS}
          createAriaLabel={createAriaLabel}
          hasUnread={hasUnread}
          onBrowse={onBrowse}
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
}: {
  section: ChannelSection;
  channels: Channel[];
  hasUnread: boolean;
  isCollapsed: boolean;
  isActiveChannel: boolean;
  selectedChannelId: string | null;
  unreadChannelIds: Set<string>;
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
  onMarkChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkSectionRead: () => void;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onUnassignChannel: (channelId: string) => void;
  onCreateSectionForChannel: (channelId: string) => void;
  onRenameSection: () => void;
  onDeleteSection: () => void;
  onMoveSectionUp: () => void;
  onMoveSectionDown: () => void;
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
                          "h-3 w-3 shrink-0 text-sidebar-foreground/30",
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
                        <CheckCheck className="h-3.5 w-3.5" />
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
                      <Pencil className="h-3.5 w-3.5" />
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
                      <Trash2 className="h-3.5 w-3.5" />
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
                            sections={sections}
                            assignments={assignments}
                            onMarkChannelRead={onMarkChannelRead}
                            onMarkChannelUnread={onMarkChannelUnread}
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
