import * as React from "react";
import { ChevronDown } from "lucide-react";

import {
  archivedHuddleRowLabel,
  groupArchivedHuddleChannels,
} from "@/app/huddleChannelVisibility";
import { SidebarSection } from "@/features/sidebar/ui/SidebarSection";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
} from "@/shared/ui/sidebar";

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "relative size-2.5 shrink-0 text-current opacity-0 transition-[color,opacity] group-hover/sidebar-section:opacity-100 group-hover/section-label:opacity-100 group-focus-within/sidebar-section:opacity-100 group-focus-visible/section-label:opacity-100";
const SECTION_LABEL_CHEVRON_ICON_CLASS =
  "absolute left-1/2 top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2";

type ArchivedHuddlesSectionProps = {
  items: Channel[];
  /** Non-ephemeral channels used to resolve parent group labels. */
  parentChannels: Channel[];
  isCollapsed?: boolean;
  isActiveChannel: boolean;
  selectedChannelId: string | null;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  mutedChannelIds?: ReadonlySet<string>;
  onMarkChannelRead?: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread?: (channelId: string) => void;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onToggleCollapsed?: () => void;
};

export function ArchivedHuddlesSection({
  items,
  parentChannels,
  isCollapsed,
  isActiveChannel,
  selectedChannelId,
  unreadChannelCounts,
  unreadChannelIds,
  mutedChannelIds,
  onMarkChannelRead,
  onMarkChannelUnread,
  onMuteChannel,
  onUnmuteChannel,
  onSelectChannel,
  onToggleCollapsed,
}: ArchivedHuddlesSectionProps) {
  const groups = React.useMemo(
    () => groupArchivedHuddleChannels(items, parentChannels),
    [items, parentChannels],
  );

  const channelLabels = React.useMemo(() => {
    const labels: Record<string, string> = {};
    for (const group of groups) {
      for (const huddle of group.channels) {
        labels[huddle.id] = archivedHuddleRowLabel(huddle);
      }
    }
    return labels;
  }, [groups]);

  const [collapsedParents, setCollapsedParents] = React.useState<
    Record<string, boolean>
  >({});

  const toggleParent = React.useCallback((key: string) => {
    setCollapsedParents((current) => ({
      ...current,
      [key]: !current[key],
    }));
  }, []);

  if (groups.length === 0) return null;

  const contentId = "sidebar-archived-huddles-list";
  const canToggle = Boolean(onToggleCollapsed);

  return (
    <SidebarGroup
      className="group/sidebar-section select-none"
      data-testid="archived-huddles-section"
    >
      <div className="relative">
        <SidebarGroupLabel asChild={canToggle}>
          {canToggle ? (
            <button
              aria-controls={contentId}
              aria-expanded={!isCollapsed}
              className={SECTION_LABEL_BUTTON_CLASS}
              data-testid="archived-huddles-list-section-label"
              onClick={onToggleCollapsed}
              type="button"
            >
              <span data-sidebar-section-title>Archived Huddles</span>
              <span aria-hidden="true" className={SECTION_LABEL_CHEVRON_CLASS}>
                <ChevronDown
                  className={cn(
                    SECTION_LABEL_CHEVRON_ICON_CLASS,
                    isCollapsed ? "-rotate-90" : "rotate-0",
                  )}
                />
              </span>
            </button>
          ) : (
            "Archived Huddles"
          )}
        </SidebarGroupLabel>
      </div>
      {!isCollapsed ? (
        <SidebarGroupContent data-testid="archived-huddles-list" id={contentId}>
          {groups.map((group) => (
            <SidebarSection
              key={group.key}
              channelLabels={channelLabels}
              isActiveChannel={isActiveChannel}
              isCollapsed={Boolean(collapsedParents[group.key])}
              items={group.channels}
              mutedChannelIds={mutedChannelIds}
              onMarkChannelRead={onMarkChannelRead}
              onMarkChannelUnread={onMarkChannelUnread}
              onMuteChannel={onMuteChannel}
              onSelectChannel={onSelectChannel}
              onToggleCollapsed={() => toggleParent(group.key)}
              onUnmuteChannel={onUnmuteChannel}
              selectedChannelId={selectedChannelId}
              testId={`archived-huddles-group-${group.key}`}
              title={group.parentLabel}
              unreadChannelCounts={unreadChannelCounts}
              unreadChannelIds={unreadChannelIds}
            />
          ))}
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}
