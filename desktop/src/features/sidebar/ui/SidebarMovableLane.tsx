import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import {
  CHANNELS_BLOCK_ID,
  SidebarDndContext,
} from "@/features/sidebar/ui/SidebarDnd";
import {
  ChannelGroupSection,
  CustomChannelSection,
} from "@/features/sidebar/ui/CustomChannelSection";
import {
  sectionSortGroupKey,
  type ChannelSortGroupKey,
  type ChannelSortMode,
} from "@/features/sidebar/lib/channelSortPreference";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import type { Channel } from "@/shared/api/types";

type SectionBuckets = {
  bySection: Record<string, Channel[]>;
  unassigned: Channel[];
};

export function SidebarMovableLane({
  blockOrder,
  channelSections,
  channelAssignments,
  sectionBuckets,
  channelGroups,
  channels,
  manualGroupKeys,
  collapsedSections,
  collapsedChannels,
  isActiveChannel,
  activeWorkingByChannelId,
  selectedChannelId,
  unreadChannelCounts,
  unreadChannelIds,
  mutedChannelIds,
  starredChannelIds,
  sortModeFor,
  handleSortModeChange,
  handleMoveChannel,
  reorderBlocks,
  moveBlockUp,
  moveBlockDown,
  assignChannel,
  unassignChannel,
  onSelectChannel,
  onMarkChannelRead,
  onMarkChannelUnread,
  onMarkAllChannelsRead,
  onBrowseChannels,
  onCreateCategory,
  onCreateSectionForChannel,
  onCreateChannelInSection,
  onRenameSection,
  onDeleteSection,
  onToggleCollapsedSection,
  onToggleCollapsedChannels,
  onMuteChannel,
  onUnmuteChannel,
  onStarChannel,
  onUnstarChannel,
  onDeleteChannel,
  onLeaveChannel,
}: {
  blockOrder: string[];
  channelSections: ChannelSection[];
  channelAssignments: Record<string, string>;
  sectionBuckets: SectionBuckets;
  channelGroups: {
    key: ChannelSortGroupKey;
    name: string;
    channelIds: string[];
  }[];
  channels: Channel[];
  manualGroupKeys: ReadonlySet<ChannelSortGroupKey>;
  collapsedSections: Record<string, boolean>;
  collapsedChannels: boolean;
  isActiveChannel: boolean;
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  selectedChannelId: string | null;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  mutedChannelIds?: ReadonlySet<string>;
  starredChannelIds?: ReadonlySet<string>;
  sortModeFor: (group: ChannelSortGroupKey) => ChannelSortMode;
  handleSortModeChange: (
    group: ChannelSortGroupKey,
    mode: ChannelSortMode,
    visible: Channel[],
  ) => void;
  handleMoveChannel: (input: {
    channelId: string;
    sourceGroup: ChannelSortGroupKey;
    targetGroup: ChannelSortGroupKey;
    overChannelId?: string;
  }) => void;
  reorderBlocks: (orderedBlockIds: string[]) => void;
  moveBlockUp: (blockId: string) => void;
  moveBlockDown: (blockId: string) => void;
  assignChannel: (channelId: string, sectionId: string) => void;
  unassignChannel: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onMarkAllChannelsRead: () => void;
  onBrowseChannels?: () => void;
  onCreateCategory: () => void;
  onCreateSectionForChannel: (channelId: string) => void;
  onCreateChannelInSection: (sectionId: string) => void;
  onRenameSection: (section: ChannelSection) => void;
  onDeleteSection: (section: ChannelSection) => void;
  onToggleCollapsedSection: (sectionId: string) => void;
  onToggleCollapsedChannels: () => void;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
  onDeleteChannel?: (channel: Channel) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  return (
    <SidebarDndContext
      channelGroups={channelGroups}
      channels={channels}
      sections={channelSections}
      blockIds={blockOrder}
      manualGroupKeys={manualGroupKeys}
      onMoveChannel={handleMoveChannel}
      onReorderBlocks={reorderBlocks}
    >
      {blockOrder.map((blockId, idx) => {
        if (blockId === CHANNELS_BLOCK_ID) {
          return (
            <ChannelGroupSection
              key={CHANNELS_BLOCK_ID}
              blockId={CHANNELS_BLOCK_ID}
              isFirstBlock={idx === 0}
              isLastBlock={idx === blockOrder.length - 1}
              onMoveBlockUp={() => moveBlockUp(CHANNELS_BLOCK_ID)}
              onMoveBlockDown={() => moveBlockDown(CHANNELS_BLOCK_ID)}
              draggable
              hasUnread={unreadChannelIds.size > 0}
              isCollapsed={collapsedChannels}
              isActiveChannel={isActiveChannel}
              activeWorkingByChannelId={activeWorkingByChannelId}
              items={sectionBuckets.unassigned}
              sortMode={sortModeFor("channels")}
              onSortModeChange={(mode) =>
                handleSortModeChange(
                  "channels",
                  mode,
                  sectionBuckets.unassigned,
                )
              }
              actionsTestId="section-actions-channels"
              listTestId="stream-list"
              quickCreateLabel="Browse channels"
              onQuickCreateClick={() => onBrowseChannels?.()}
              showQuickCreate
              onMarkAllRead={onMarkAllChannelsRead}
              onMarkChannelRead={onMarkChannelRead}
              onMarkChannelUnread={onMarkChannelUnread}
              onSelectChannel={onSelectChannel}
              onToggleCollapsed={onToggleCollapsedChannels}
              selectedChannelId={selectedChannelId}
              title="Channels"
              unreadChannelCounts={unreadChannelCounts}
              unreadChannelIds={unreadChannelIds}
              sections={channelSections}
              assignments={channelAssignments}
              onAssignChannel={assignChannel}
              onUnassignChannel={unassignChannel}
              onCreateSectionForChannel={onCreateSectionForChannel}
              onCreateCategory={onCreateCategory}
              groupKey="channels"
              manualSortEnabled
              mutedChannelIds={mutedChannelIds}
              onMuteChannel={onMuteChannel}
              onUnmuteChannel={onUnmuteChannel}
              starredChannelIds={starredChannelIds}
              onStarChannel={onStarChannel}
              onUnstarChannel={onUnstarChannel}
              onDeleteChannel={onDeleteChannel}
              onLeaveChannel={onLeaveChannel}
            />
          );
        }
        const section = channelSections.find((s) => s.id === blockId);
        if (!section) return null;
        const sectionChannels = sectionBuckets.bySection[section.id] ?? [];
        return (
          <CustomChannelSection
            key={section.id}
            section={section}
            channels={sectionChannels}
            hasUnread={sectionChannels.some((c) => unreadChannelIds.has(c.id))}
            isCollapsed={collapsedSections[section.id] ?? false}
            isActiveChannel={isActiveChannel}
            activeWorkingByChannelId={activeWorkingByChannelId}
            selectedChannelId={selectedChannelId}
            unreadChannelCounts={unreadChannelCounts}
            unreadChannelIds={unreadChannelIds}
            sections={channelSections}
            assignments={channelAssignments}
            isFirst={idx === 0}
            isLast={idx === blockOrder.length - 1}
            sortMode={sortModeFor(sectionSortGroupKey(section.id))}
            onSortModeChange={(mode) =>
              handleSortModeChange(
                sectionSortGroupKey(section.id),
                mode,
                sectionChannels,
              )
            }
            onToggleCollapsed={() => onToggleCollapsedSection(section.id)}
            onSelectChannel={onSelectChannel}
            onMarkChannelRead={onMarkChannelRead}
            onMarkChannelUnread={onMarkChannelUnread}
            onMarkSectionRead={() => {
              for (const channel of sectionChannels) {
                onMarkChannelRead(channel.id, channel.lastMessageAt);
              }
            }}
            onAssignChannel={assignChannel}
            onUnassignChannel={unassignChannel}
            onCreateSectionForChannel={onCreateSectionForChannel}
            onCreateChannel={() => onCreateChannelInSection(section.id)}
            onRenameSection={() => onRenameSection(section)}
            onDeleteSection={() => onDeleteSection(section)}
            onMoveSectionUp={() => moveBlockUp(section.id)}
            onMoveSectionDown={() => moveBlockDown(section.id)}
            mutedChannelIds={mutedChannelIds}
            onMuteChannel={onMuteChannel}
            onUnmuteChannel={onUnmuteChannel}
            starredChannelIds={starredChannelIds}
            onStarChannel={onStarChannel}
            onUnstarChannel={onUnstarChannel}
            onDeleteChannel={onDeleteChannel}
            onLeaveChannel={onLeaveChannel}
          />
        );
      })}
    </SidebarDndContext>
  );
}
