import type { ChannelProjectFeature } from "@/features/projects/channelProjectFeatures";
import { cn } from "@/shared/lib/cn";
import * as React from "react";

export type ProjectChannelView =
  | "chat"
  | "canvas"
  | "issues"
  | "channels"
  | "reviews"
  | "repos";

const PROJECT_CHANNEL_CHAT_TAB = {
  label: "Chat",
  testId: "project-channel-tab-chat",
  value: "chat",
} as const;

const PROJECT_CHANNEL_EXTRA_TABS: Array<{
  feature?: ChannelProjectFeature;
  label: string;
  testId: string;
  value: Exclude<ProjectChannelView, "chat">;
}> = [
  {
    label: "Canvas",
    testId: "project-channel-tab-canvas",
    value: "canvas",
  },
  {
    feature: "tasks",
    label: "Tasks",
    testId: "project-channel-tab-tasks",
    value: "issues",
  },
  {
    feature: "breakouts",
    label: "Channels",
    testId: "project-channel-tab-channels",
    value: "channels",
  },
  {
    feature: "reviews",
    label: "Reviews",
    testId: "project-channel-tab-reviews",
    value: "reviews",
  },
  {
    feature: "repositories",
    label: "Repos",
    testId: "project-channel-tab-repos",
    value: "repos",
  },
];

export function projectChannelViewEnabled(
  view: ProjectChannelView,
  enabledFeatures: Record<ChannelProjectFeature, boolean>,
) {
  if (view === "chat" || view === "canvas") return true;
  const tab = PROJECT_CHANNEL_EXTRA_TABS.find(
    (candidate) => candidate.value === view,
  );
  return tab?.feature ? enabledFeatures[tab.feature] : false;
}

export function ProjectChannelTabs({
  activeView,
  enabledFeatures,
  onSelect,
}: {
  activeView: ProjectChannelView;
  enabledFeatures: Record<ChannelProjectFeature, boolean>;
  onSelect: (view: ProjectChannelView) => void;
}) {
  const activeTabRef = React.useRef<HTMLButtonElement>(null);
  const extraTabs = PROJECT_CHANNEL_EXTRA_TABS.filter(
    (tab) => !tab.feature || enabledFeatures[tab.feature],
  );
  const setActiveTabRef = React.useCallback((tab: HTMLButtonElement | null) => {
    activeTabRef.current = tab;
    tab?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, []);

  React.useEffect(() => {
    const revealActiveTab = () => {
      activeTabRef.current?.scrollIntoView({
        block: "nearest",
        inline: "nearest",
      });
    };
    window.addEventListener("resize", revealActiveTab);
    return () => window.removeEventListener("resize", revealActiveTab);
  }, []);

  if (extraTabs.length === 0) return null;

  const tabs = [PROJECT_CHANNEL_CHAT_TAB, ...extraTabs];

  return (
    <div
      aria-label="Channel views"
      className="flex h-9 min-w-max items-stretch"
      data-testid="project-channel-tabs"
      role="tablist"
    >
      {tabs.map((tab) => (
        <button
          aria-selected={activeView === tab.value}
          className={cn(
            "relative h-9 shrink-0 px-2 text-xs font-medium text-muted-foreground outline-hidden transition-colors after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:bg-transparent after:content-[''] hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
            activeView === tab.value && "text-foreground after:bg-foreground",
          )}
          data-testid={tab.testId}
          key={tab.value}
          onClick={() => onSelect(tab.value)}
          ref={activeView === tab.value ? setActiveTabRef : undefined}
          role="tab"
          type="button"
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
