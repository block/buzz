import type { Channel } from "@/shared/api/types";
import { isMacPlatform, isWindowsPlatform } from "@/shared/lib/platform";

import { useTrayMenu } from "@/app/useTrayMenu";

/** Keeps the ticking native tray menu outside AppShell's render cycle. */
export function AppShellTrayMenu({
  channels,
  goChannel,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
}) {
  if (!isMacPlatform() && !isWindowsPlatform()) return null;
  return <NativeAppShellTrayMenu channels={channels} goChannel={goChannel} />;
}

function NativeAppShellTrayMenu({
  channels,
  goChannel,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
}): null {
  useTrayMenu({
    channels,
    goChannel,
  });
  return null;
}
