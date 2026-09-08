export { TerminalBootstrap } from "@/features/terminal/TerminalBootstrap";
import * as React from "react";

import type { Channel } from "@/shared/api/types";
import type { CreateChannelInput } from "@/features/sidebar/lib/useCreateChannelForm";
import { useDeferredModalOpen } from "@/shared/ui/deferredModalOpen";
import {
  mergeOpenChannelDirectory,
  useOpenChannelDirectoryQuery,
} from "@/features/channels/openChannelDirectory";

const ChannelBrowserDialog = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelBrowserDialog");
  return { default: module.ChannelBrowserDialog };
});

const ChannelManagementSheet = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelManagementSheet");
  return { default: module.ChannelManagementSheet };
});

const MembersSidebar = React.lazy(async () => {
  const module = await import("@/features/channels/ui/MembersSidebar");
  return { default: module.MembersSidebar };
});

export type BrowseDialogType = "stream" | "forum" | null;

type AppShellOverlaysProps = {
  activeChannel: Channel | null;
  browseDialogType: BrowseDialogType;
  channels: Channel[];
  currentPubkey?: string;
  isChannelManagementOpen: boolean;
  isCreatingBrowseChannel?: boolean;
  onBrowseChannelJoin: (channelId: string) => Promise<void>;
  onBrowseChannelCreate?: (input: CreateChannelInput) => Promise<void>;
  onBrowseDialogOpenChange: (open: boolean) => void;
  onChannelManagementOpenChange: (open: boolean) => void;
  onDeleteActiveChannel: () => void;
  onSelectChannel: (channelId: string) => void;
  relayUrl?: string;
};

export function AppShellOverlays({
  activeChannel,
  browseDialogType,
  channels,
  currentPubkey,
  isChannelManagementOpen,
  isCreatingBrowseChannel,
  onBrowseChannelJoin,
  onBrowseChannelCreate,
  onBrowseDialogOpenChange,
  onChannelManagementOpenChange,
  onDeleteActiveChannel,
  onSelectChannel,
  relayUrl,
}: AppShellOverlaysProps) {
  const [membersChannel, setMembersChannel] = React.useState<Channel | null>(
    null,
  );
  const [visibleBrowseDialogType, setVisibleBrowseDialogType] =
    React.useState<BrowseDialogType>(null);
  const { cancelDeferredModalOpen, openNextFrame: openModalNextFrame } =
    useDeferredModalOpen();

  React.useEffect(() => {
    if (browseDialogType === null) {
      cancelDeferredModalOpen();
      setVisibleBrowseDialogType(null);
      return;
    }

    setVisibleBrowseDialogType(null);
    openModalNextFrame(() => {
      setVisibleBrowseDialogType(browseDialogType);
    });
  }, [browseDialogType, cancelDeferredModalOpen, openModalNextFrame]);

  const renderedBrowseDialogType = visibleBrowseDialogType ?? browseDialogType;

  // The channel browser is the only overlay that shows non-member open
  // channels, so it — not the 60s poll — pays for the all-open directory scan,
  // and only while it is open. Merge the superset over the member list so a
  // just-joined or optimistic channel keeps its live state.
  const openDirectoryQuery = useOpenChannelDirectoryQuery({
    enabled: browseDialogType !== null,
  });
  const browserChannels = React.useMemo(
    () => mergeOpenChannelDirectory(channels, openDirectoryQuery.data),
    [channels, openDirectoryQuery.data],
  );

  return (
    <>
      {browseDialogType !== null ? (
        <React.Suspense fallback={null}>
          <ChannelBrowserDialog
            channels={browserChannels}
            channelTypeFilter={renderedBrowseDialogType ?? browseDialogType}
            isCreatingChannel={isCreatingBrowseChannel}
            onCreateChannel={onBrowseChannelCreate}
            onJoinChannel={onBrowseChannelJoin}
            onOpenChange={onBrowseDialogOpenChange}
            onSelectChannel={onSelectChannel}
            open={visibleBrowseDialogType !== null}
          />
        </React.Suspense>
      ) : null}

      {isChannelManagementOpen && activeChannel !== null ? (
        <React.Suspense fallback={null}>
          <ChannelManagementSheet
            channel={activeChannel}
            currentPubkey={currentPubkey}
            onDeleted={onDeleteActiveChannel}
            onOpenMembers={() => setMembersChannel(activeChannel)}
            onOpenChange={onChannelManagementOpenChange}
            open={true}
          />
        </React.Suspense>
      ) : null}

      {membersChannel ? (
        <React.Suspense fallback={null}>
          <MembersSidebar
            channel={membersChannel}
            currentPubkey={currentPubkey}
            onOpenChange={(nextOpen) => {
              if (!nextOpen) {
                setMembersChannel(null);
              }
            }}
            open={true}
            relayUrl={relayUrl}
          />
        </React.Suspense>
      ) : null}
    </>
  );
}
