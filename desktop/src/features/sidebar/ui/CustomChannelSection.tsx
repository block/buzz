import {
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  CheckCheck,
  ChevronDown,
  EllipsisVertical,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";

import { useRef, useState } from "react";
import type * as React from "react";

import type { ChannelSortMode } from "@/features/sidebar/lib/channelSortPreference";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
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
import { ChannelContextMenuItems } from "@/features/sidebar/ui/ChannelContextMenu";
import { deferMenuAction } from "@/features/sidebar/ui/sidebarMenuHelpers";
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
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { getPlatformKeysById } from "@/shared/lib/keyboard-shortcuts";
import { HashSearch } from "@/shared/ui/icons";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "relative size-2.5 shrink-0 text-current opacity-0 transition-[color,opacity] group-hover/sidebar-section:opacity-100 group-hover/section-label:opacity-100 group-focus-within/sidebar-section:opacity-100 group-focus-visible/section-label:opacity-100 group-data-[section-actions-open=true]/sidebar-section:opacity-100";
const SECTION_LABEL_CHEVRON_ICON_CLASS =
  "absolute left-1/2 top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2";

const SORT_OPTIONS: { value: ChannelSortMode; label: string }[] = [
  { value: "recent", label: "Recent" },
  { value: "alpha", label: "A–Z" },
];

/**
 * A single always-visible "+" quick action shown at the right edge of a
 * section header, to the left of the ⋮ menu. Used for the most common
 * per-section create action (New channel, New message) while every other
 * action stays folded into {@link SectionActionsMenu}.
 */
export function SectionQuickAction({
  label,
  onClick,
  testId,
  icon: Icon = Plus,
  visibilityClassName = SECTION_ACTION_VISIBILITY_CLASS,
}: {
  label: string;
  onClick: () => void;
  testId?: string;
  icon?: typeof Plus;
  visibilityClassName?: string;
}) {
  return (
    <button
      aria-label={label}
      className={cn(SECTION_ICON_BUTTON_CLASS, visibilityClassName)}
      data-testid={testId}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
      onPointerDown={(event) => event.stopPropagation()}
      title={label}
      type="button"
    >
      <Icon className="h-4 w-4" />
    </button>
  );
}

/**
 * The single "more actions" menu shown at the right edge of every sidebar
 * section header (Starred, Channels, Forums, Direct messages, and each custom
 * section). Items render only when their handler is provided; a Sort radio
 * group is appended whenever a sort preference is supplied.
 */
export function SectionActionsMenu({
  sectionLabel,
  testId,
  visibilityClassName = SECTION_ACTION_VISIBILITY_CLASS,
  onOpenChange,
  hasUnread,
  onMarkAllRead,
  onBrowse,
  browseLabel,
  onCreate,
  createLabel,
  onNewMessage,
  newMessageLabel,
  onRenameSection,
  onMoveSectionUp,
  onMoveSectionDown,
  onDeleteSection,
  isFirstSection,
  isLastSection,
  sortMode,
  onSortModeChange,
}: {
  sectionLabel: string;
  testId?: string;
  visibilityClassName?: string;
  onOpenChange?: (open: boolean) => void;
  hasUnread?: boolean;
  onMarkAllRead?: () => void;
  onBrowse?: () => void;
  browseLabel?: string;
  onCreate?: () => void;
  createLabel?: string;
  onNewMessage?: () => void;
  newMessageLabel?: string;
  onRenameSection?: () => void;
  onMoveSectionUp?: () => void;
  onMoveSectionDown?: () => void;
  onDeleteSection?: () => void;
  isFirstSection?: boolean;
  isLastSection?: boolean;
  sortMode?: ChannelSortMode;
  onSortModeChange?: (mode: ChannelSortMode) => void;
}) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const showSectionManagement = Boolean(onRenameSection || onDeleteSection);
  const showSort = Boolean(sortMode && onSortModeChange);

  return (
    <DropdownMenu onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`More actions for ${sectionLabel}`}
          className={cn(SECTION_ICON_BUTTON_CLASS, visibilityClassName)}
          data-testid={testId}
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
        {hasUnread && onMarkAllRead ? (
          <DropdownMenuItem onSelect={() => deferMenuAction(onMarkAllRead)}>
            <CheckCheck className="h-4 w-4" />
            <span>Mark all as read</span>
          </DropdownMenuItem>
        ) : null}
        {onNewMessage ? (
          <DropdownMenuItem onSelect={() => deferMenuAction(onNewMessage)}>
            <Plus className="h-4 w-4" />
            <span>{newMessageLabel ?? "New message"}</span>
          </DropdownMenuItem>
        ) : null}
        {onBrowse ? (
          <DropdownMenuItem onSelect={() => deferMenuAction(onBrowse)}>
            <HashSearch className="h-4 w-4" />
            <span>{browseLabel ?? "Browse channels"}</span>
            <DropdownMenuShortcut>
              {getPlatformKeysById("browse-channels")}
            </DropdownMenuShortcut>
          </DropdownMenuItem>
        ) : null}
        {onCreate ? (
          <DropdownMenuItem onSelect={() => deferMenuAction(onCreate)}>
            <Plus className="h-4 w-4" />
            <span>{createLabel ?? "Create channel"}</span>
          </DropdownMenuItem>
        ) : null}
        {showSectionManagement ? (
          <>
            {onRenameSection ? (
              <DropdownMenuItem
                onSelect={() => deferMenuAction(onRenameSection)}
              >
                <Pencil className="h-4 w-4" />
                <span>Rename section</span>
              </DropdownMenuItem>
            ) : null}
            {onMoveSectionUp ? (
              <DropdownMenuItem
                disabled={isFirstSection}
                onSelect={() => deferMenuAction(onMoveSectionUp)}
              >
                <ArrowUp className="h-4 w-4" />
                <span>Move up</span>
              </DropdownMenuItem>
            ) : null}
            {onMoveSectionDown ? (
              <DropdownMenuItem
                disabled={isLastSection}
                onSelect={() => deferMenuAction(onMoveSectionDown)}
              >
                <ArrowDown className="h-4 w-4" />
                <span>Move down</span>
              </DropdownMenuItem>
            ) : null}
          </>
        ) : null}
        {showSort ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <ArrowUpDown className="h-4 w-4" />
                <span>Sort</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent>
                <DropdownMenuRadioGroup
                  onValueChange={(value) =>
                    onSortModeChange?.(value as ChannelSortMode)
                  }
                  value={sortMode}
                >
                  {SORT_OPTIONS.map((option) => (
                    <DropdownMenuRadioItem
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          </>
        ) : null}
        {onDeleteSection ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              onSelect={() => deferMenuAction(onDeleteSection)}
            >
              <Trash2 className="h-4 w-4" />
              <span>Delete section</span>
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ChannelSectionHeader({
  contentId,
  isCollapsed,
  onToggleCollapsed,
  title,
  testId,
  actions,
}: {
  contentId: string;
  isCollapsed: boolean;
  onToggleCollapsed: () => void;
  title: string;
  testId: string;
  actions: React.ReactNode;
}) {
  return (
    <div className="relative">
      <SidebarGroupLabel asChild>
        <button
          aria-controls={contentId}
          aria-expanded={!isCollapsed}
          className={SECTION_LABEL_BUTTON_CLASS}
          data-testid={`${testId}-section-label`}
          onClick={onToggleCollapsed}
          type="button"
        >
          <span data-sidebar-section-title>{title}</span>
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
      <div className="absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5">
        {actions}
      </div>
    </div>
  );
}

export function ChannelGroupSection({
  browseLabel,
  createLabel,
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
  onQuickCreateClick,
  quickCreateLabel,
  showQuickCreate,
  onMarkAllRead,
  onMarkChannelRead,
  onMarkChannelUnread,
  onSelectChannel,
  onToggleCollapsed,
  selectedChannelId,
  sortMode,
  onSortModeChange,
  actionsTestId,
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
  onDeleteChannel,
  onLeaveChannel,
}: {
  browseLabel?: string;
  createLabel?: string;
  draggable?: boolean;
  groupClassName?: string;
  isCollapsed: boolean;
  isActiveChannel: boolean;
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  items: Channel[];
  listTestId: string;
  onBrowseClick?: () => void;
  onCreateClick?: () => void;
  /**
   * Overrides the quick-create (`+`) button's click handler. Defaults to
   * `onCreateClick`. Used to point the sidebar `+` at the unified
   * "Add channel" search-and-create browser instead of the bare create form.
   */
  onQuickCreateClick?: () => void;
  /** Overrides the quick-create button's aria-label/tooltip. */
  quickCreateLabel?: string;
  showQuickCreate?: boolean;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onToggleCollapsed: () => void;
  selectedChannelId: string | null;
  sortMode?: ChannelSortMode;
  onSortModeChange?: (mode: ChannelSortMode) => void;
  actionsTestId?: string;
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
  onDeleteChannel?: (channel: Channel) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const contentId = `sidebar-${listTestId}`;
  const [actionsMenuOpen, setActionsMenuOpen] = useState(false);

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
                onDeleteChannel={onDeleteChannel}
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
      data-section-actions-open={actionsMenuOpen || undefined}
    >
      <ChannelSectionHeader
        contentId={contentId}
        isCollapsed={isCollapsed}
        onToggleCollapsed={onToggleCollapsed}
        title={title}
        testId={listTestId}
        actions={
          <>
            {showQuickCreate && (onQuickCreateClick ?? onCreateClick) ? (
              <SectionQuickAction
                label={quickCreateLabel ?? createLabel ?? "Create channel"}
                onClick={(onQuickCreateClick ?? onCreateClick) as () => void}
                testId={
                  actionsTestId ? `${actionsTestId}-quick-create` : undefined
                }
              />
            ) : null}
            <SectionActionsMenu
              sectionLabel={title}
              testId={actionsTestId}
              onOpenChange={setActionsMenuOpen}
              hasUnread={hasUnread}
              onMarkAllRead={onMarkAllRead}
              onBrowse={onBrowseClick}
              browseLabel={browseLabel}
              onCreate={onCreateClick}
              createLabel={createLabel}
              sortMode={sortMode}
              onSortModeChange={onSortModeChange}
            />
          </>
        }
      />
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
  sortMode,
  onSortModeChange,
  onToggleCollapsed,
  onSelectChannel,
  onMarkChannelRead,
  onMarkChannelUnread,
  onMarkSectionRead,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
  onCreateChannel,
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
  onDeleteChannel,
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
  sortMode?: ChannelSortMode;
  onSortModeChange?: (mode: ChannelSortMode) => void;
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
  onCreateChannel: () => void;
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
  onDeleteChannel?: (channel: Channel) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const contentId = `sidebar-section-${section.id}`;
  const [actionsMenuOpen, setActionsMenuOpen] = useState(false);

  return (
    <SortableSectionShell sectionId={section.id}>
      {({ dragHandleProps, isDragging }) => (
        <DroppableSectionBody sectionId={section.id}>
          <SidebarGroup
            className={cn(
              "group/sidebar-section select-none",
              isDragging && "opacity-30",
            )}
            data-section-actions-open={actionsMenuOpen || undefined}
          >
            <ContextMenu>
              <ContextMenuTrigger asChild>
                <div className="relative" {...dragHandleProps}>
                  <SidebarGroupLabel
                    asChild
                    className={section.icon ? undefined : "pl-8"}
                  >
                    <button
                      aria-controls={contentId}
                      aria-expanded={!isCollapsed}
                      className={cn(
                        SECTION_LABEL_BUTTON_CLASS,
                        section.icon && "gap-2",
                      )}
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
                        data-sidebar-section-title
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
                    <SectionQuickAction
                      label={`Add channel to ${section.name}`}
                      onClick={onCreateChannel}
                      testId={`section-actions-${section.id}-quick-create`}
                    />
                    <SectionActionsMenu
                      sectionLabel={section.name}
                      testId={`section-actions-${section.id}`}
                      onOpenChange={setActionsMenuOpen}
                      hasUnread={hasUnread}
                      onMarkAllRead={onMarkSectionRead}
                      onRenameSection={onRenameSection}
                      onMoveSectionUp={onMoveSectionUp}
                      onMoveSectionDown={onMoveSectionDown}
                      onDeleteSection={onDeleteSection}
                      isFirstSection={isFirst}
                      isLastSection={isLast}
                      sortMode={sortMode}
                      onSortModeChange={onSortModeChange}
                    />
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
                            onDeleteChannel={onDeleteChannel}
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
