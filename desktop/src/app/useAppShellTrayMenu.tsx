import type { Channel } from "@/shared/api/types";
import { isMacPlatform } from "@/shared/lib/platform";

import { useTrayMenu } from "@/app/useTrayMenu";

/** Keeps the ticking native tray menu outside AppShell's render cycle. */
export function AppShellTrayMenu({
  channels,
  goChannel,
  openCreateChannel,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  openCreateChannel: () => void;
}) {
  if (!isMacPlatform()) return null;
  return (
    <MacAppShellTrayMenu
      channels={channels}
      goChannel={goChannel}
      openCreateChannel={openCreateChannel}
    />
  );
}

function MacAppShellTrayMenu({
  channels,
  goChannel,
  openCreateChannel,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  openCreateChannel: () => void;
}): null {
  useTrayMenu({
    channels,
    goChannel,
    openCreateChannel,
  });
  return null;
}
