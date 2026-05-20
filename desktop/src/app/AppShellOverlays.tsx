import * as React from "react";

import type { Channel, SearchHit } from "@/shared/api/types";

const ChannelBrowserDialog = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelBrowserDialog");
  return { default: module.ChannelBrowserDialog };
});

const ChannelManagementSheet = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelManagementSheet");
  return { default: module.ChannelManagementSheet };
});

const SearchDialog = React.lazy(async () => {
  const module = await import("@/features/search/ui/SearchDialog");
  return { default: module.SearchDialog };
});

export type BrowseDialogType = "stream" | "forum" | null;

type AppShellOverlaysProps = {
  activeChannel: Channel | null;
  browseDialogType: BrowseDialogType;
  channels: Channel[];
  currentPubkey?: string;
  isChannelManagementOpen: boolean;
  isSearchOpen: boolean;
  onBrowseChannelJoin: (channelId: string) => Promise<void>;
  onBrowseDialogOpenChange: (open: boolean) => void;
  onChannelManagementOpenChange: (open: boolean) => void;
  onDeleteActiveChannel: () => void;
  onOpenSearchResult: (hit: SearchHit) => void;
  onSearchOpenChange: (open: boolean) => void;
  onSelectChannel: (channelId: string) => void;
};

export function AppShellOverlays({
  activeChannel,
  browseDialogType,
  channels,
  currentPubkey,
  isChannelManagementOpen,
  isSearchOpen,
  onBrowseChannelJoin,
  onBrowseDialogOpenChange,
  onChannelManagementOpenChange,
  onDeleteActiveChannel,
  onOpenSearchResult,
  onSearchOpenChange,
  onSelectChannel,
}: AppShellOverlaysProps) {
  return (
    <>
      {browseDialogType !== null ? (
        <React.Suspense fallback={null}>
          <ChannelBrowserDialog
            channels={channels}
            channelTypeFilter={browseDialogType}
            onJoinChannel={onBrowseChannelJoin}
            onOpenChange={onBrowseDialogOpenChange}
            onSelectChannel={onSelectChannel}
            open={true}
          />
        </React.Suspense>
      ) : null}

      {isSearchOpen ? (
        <React.Suspense fallback={null}>
          <SearchDialog
            channels={channels}
            currentPubkey={currentPubkey}
            onOpenChannel={onSelectChannel}
            onOpenResult={onOpenSearchResult}
            onOpenChange={onSearchOpenChange}
            open={true}
          />
        </React.Suspense>
      ) : null}

      {isChannelManagementOpen && activeChannel !== null ? (
        <React.Suspense fallback={null}>
          <ChannelManagementSheet
            channel={activeChannel}
            currentPubkey={currentPubkey}
            onDeleted={onDeleteActiveChannel}
            onOpenChange={onChannelManagementOpenChange}
            open={true}
          />
        </React.Suspense>
      ) : null}
    </>
  );
}
