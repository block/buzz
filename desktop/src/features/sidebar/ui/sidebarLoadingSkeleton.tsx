import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { Skeleton } from "@/shared/ui/skeleton";

const SIDEBAR_SKELETON_CACHE_PREFIX = "buzz-sidebar-skeleton-shape.v1";
const sidebarLoadingWidthClasses = [
  "w-14",
  "w-16",
  "w-20",
  "w-24",
  "w-28",
  "w-32",
] as const;

type SidebarLoadingWidthClass = (typeof sidebarLoadingWidthClasses)[number];

type SidebarLoadingRowShape = {
  avatar?: boolean;
  key: string;
  unread?: boolean;
  widthClass: SidebarLoadingWidthClass;
};

type SidebarLoadingShape = {
  channels: SidebarLoadingRowShape[];
  directMessages: SidebarLoadingRowShape[];
};

type SidebarLoadingCachePayload = SidebarLoadingShape & {
  version: 1;
};

const fallbackSidebarLoadingShape: SidebarLoadingShape = {
  channels: [
    { key: "agents", widthClass: "w-20" },
    { key: "engineering", widthClass: "w-28" },
    { key: "general", widthClass: "w-20" },
  ],
  directMessages: [
    { key: "alice", widthClass: "w-24" },
    { key: "bob", widthClass: "w-20" },
  ],
};

function isSidebarLoadingWidthClass(
  value: unknown,
): value is SidebarLoadingWidthClass {
  return sidebarLoadingWidthClasses.includes(value as SidebarLoadingWidthClass);
}

function parseSidebarLoadingRows(
  rows: unknown,
  maxRows: number,
): SidebarLoadingRowShape[] {
  if (!Array.isArray(rows)) return [];

  return rows
    .slice(0, maxRows)
    .filter((row: unknown): row is SidebarLoadingRowShape => {
      if (typeof row !== "object" || row === null) return false;
      const record = row as Record<string, unknown>;
      return (
        typeof record.key === "string" &&
        isSidebarLoadingWidthClass(record.widthClass) &&
        (record.unread === undefined || typeof record.unread === "boolean") &&
        (record.avatar === undefined || typeof record.avatar === "boolean")
      );
    });
}

function parseSidebarLoadingShape(value: unknown): SidebarLoadingShape | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.version !== 1) return null;

  const shape = {
    channels: parseSidebarLoadingRows(record.channels, 3),
    directMessages: parseSidebarLoadingRows(record.directMessages, 2),
  };

  return hasSidebarLoadingRows(shape) ? shape : null;
}

function hasSidebarLoadingRows(shape: SidebarLoadingShape) {
  return shape.channels.length > 0 || shape.directMessages.length > 0;
}

function sidebarSkeletonCacheKey(
  workspaceId: string | null | undefined,
  pubkey: string | undefined,
) {
  if (!workspaceId) return null;
  return `${SIDEBAR_SKELETON_CACHE_PREFIX}:${workspaceId}:${pubkey ?? "anonymous"}`;
}

function readSidebarLoadingShape(
  cacheKey: string | null,
): SidebarLoadingShape | null {
  if (!cacheKey || typeof window === "undefined") return null;

  try {
    const raw = window.localStorage.getItem(cacheKey);
    return raw ? parseSidebarLoadingShape(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

function writeSidebarLoadingShape(
  cacheKey: string | null,
  shape: SidebarLoadingShape,
) {
  if (
    !cacheKey ||
    !hasSidebarLoadingRows(shape) ||
    typeof window === "undefined"
  ) {
    return;
  }

  const payload: SidebarLoadingCachePayload = {
    channels: shape.channels.slice(0, 3),
    directMessages: shape.directMessages.slice(0, 2),
    version: 1,
  };

  try {
    window.localStorage.setItem(cacheKey, JSON.stringify(payload));
  } catch {
    // localStorage can be unavailable or full in embedded webviews.
  }
}

function sidebarWidthClassForText(text: string): SidebarLoadingWidthClass {
  const length = text.trim().length;
  if (length >= 20) return "w-32";
  if (length >= 14) return "w-28";
  if (length >= 10) return "w-24";
  if (length >= 6) return "w-20";
  return "w-16";
}

function createSidebarLoadingShape({
  directMessages,
  dmChannelLabels,
  streamChannels,
}: {
  directMessages: Channel[];
  dmChannelLabels: Record<string, string>;
  streamChannels: Channel[];
}): SidebarLoadingShape {
  return {
    channels: streamChannels.slice(0, 3).map((channel) => ({
      key: channel.id,
      widthClass: sidebarWidthClassForText(channel.name),
    })),
    directMessages: directMessages.slice(0, 2).map((channel) => ({
      avatar: true,
      key: channel.id,
      widthClass: sidebarWidthClassForText(
        dmChannelLabels[channel.id] ?? channel.name,
      ),
    })),
  };
}

export function useSidebarLoadingShape({
  activeWorkspaceId,
  directMessages,
  dmChannelLabels,
  isLoading,
  currentPubkey,
  streamChannels,
}: {
  activeWorkspaceId: string | null | undefined;
  directMessages: Channel[];
  dmChannelLabels: Record<string, string>;
  isLoading: boolean;
  currentPubkey?: string;
  streamChannels: Channel[];
}) {
  const cacheKey = React.useMemo(
    () => sidebarSkeletonCacheKey(activeWorkspaceId, currentPubkey),
    [activeWorkspaceId, currentPubkey],
  );
  const liveShape = React.useMemo(
    () =>
      createSidebarLoadingShape({
        directMessages,
        dmChannelLabels,
        streamChannels,
      }),
    [directMessages, dmChannelLabels, streamChannels],
  );
  const cachedShape = React.useMemo(
    () => readSidebarLoadingShape(cacheKey),
    [cacheKey],
  );

  React.useEffect(() => {
    if (isLoading || !hasSidebarLoadingRows(liveShape)) return;
    writeSidebarLoadingShape(cacheKey, liveShape);
  }, [cacheKey, isLoading, liveShape]);

  if (hasSidebarLoadingRows(liveShape)) return liveShape;
  return cachedShape ?? fallbackSidebarLoadingShape;
}

function SidebarLoadingRow({
  avatar = false,
  widthClass,
}: {
  avatar?: boolean;
  widthClass: string;
}) {
  return (
    <SidebarMenuItem>
      <div className="flex h-8 items-center gap-2 rounded-md px-2">
        <Skeleton
          className={cn(
            "shrink-0",
            avatar ? "h-5 w-5 rounded-full" : "h-4 w-4 rounded-sm",
          )}
        />
        <Skeleton className={cn("h-4 min-w-0", widthClass)} />
      </div>
    </SidebarMenuItem>
  );
}

function SidebarLoadingSection({
  children,
  titleWidthClass,
}: {
  children: React.ReactNode;
  titleWidthClass: string;
}) {
  return (
    <SidebarGroup>
      <div className="group/sidebar-section relative">
        <SidebarGroupLabel asChild>
          <div className="flex h-7 w-fit max-w-[calc(100%-3rem)] items-center gap-1">
            <Skeleton className={cn("h-3.5", titleWidthClass)} />
          </div>
        </SidebarGroupLabel>
      </div>
      <SidebarGroupContent>
        <SidebarMenu>{children}</SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

export function SidebarLoadingContent({
  shape,
}: {
  shape: SidebarLoadingShape;
}) {
  return (
    <div data-testid="sidebar-loading">
      <SidebarLoadingSection titleWidthClass="w-16">
        {shape.channels.map((row) => (
          <SidebarLoadingRow key={row.key} widthClass={row.widthClass} />
        ))}
      </SidebarLoadingSection>
      <SidebarLoadingSection titleWidthClass="w-24">
        {shape.directMessages.map((row) => (
          <SidebarLoadingRow avatar key={row.key} widthClass={row.widthClass} />
        ))}
      </SidebarLoadingSection>
    </div>
  );
}
