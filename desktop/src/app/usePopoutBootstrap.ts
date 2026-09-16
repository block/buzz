import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  getActiveEmbeddedWindow,
  getEmbeddedWindowsStore,
  subscribeEmbeddedWindows,
} from "@/features/popout/lib/embeddedWindows";
import {
  getPopoutSettings,
  isEmbedInMainEnabled,
  subscribePopoutSettings,
} from "@/features/popout/lib/popoutSettings";
import {
  currentPopoutPayload,
  type PopoutPayload,
} from "@/features/popout/lib/popoutWindow";

export function usePopoutBootstrap(): PopoutPayload | null {
  const payload = React.useMemo(() => currentPopoutPayload(), []);
  const { goChannel } = useAppNavigation();

  React.useEffect(() => {
    if (!payload || payload.kind === "playground") return;
    if (!payload.channelId) return;
    void goChannel(payload.channelId, {
      replace: true,
      ...(payload.threadId ? { thread: payload.threadId } : {}),
    });
  }, [goChannel, payload]);

  useEmbeddedWindowBootstrap();

  return payload;
}

function useEmbeddedWindowBootstrap() {
  React.useSyncExternalStore(
    subscribePopoutSettings,
    getPopoutSettings,
    getPopoutSettings,
  );
  React.useSyncExternalStore(
    subscribeEmbeddedWindows,
    getEmbeddedWindowsStore,
    getEmbeddedWindowsStore,
  );
  const embed = isEmbedInMainEnabled() ? getActiveEmbeddedWindow() : null;
  const { goChannel } = useAppNavigation();
  const lastLabel = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!embed || embed.payload.kind === "playground") {
      lastLabel.current = embed?.label ?? null;
      return;
    }
    if (!embed.payload.channelId) return;
    if (lastLabel.current === embed.label) return;
    lastLabel.current = embed.label;
    void goChannel(embed.payload.channelId, {
      ...(embed.payload.threadId ? { thread: embed.payload.threadId } : {}),
    });
  }, [embed, goChannel]);
}
