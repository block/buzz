// Mounted once in AppShell: keeps the remote-access status fresh and shows a local notification when a remote terminal opens.
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";

import { refreshRemoteAccess, subscribeRemoteAccess } from "./remoteAccessStore";

export function useRemoteAccessEvents(): void {
  React.useEffect(() => {
    if (!isTauri()) return;
    const unsubStore = subscribeRemoteAccess(() => {});
    const unlistens: Array<() => void> = [];
    let cancelled = false;
    void listen("remote-access-changed", () => void refreshRemoteAccess()).then((u) => (cancelled ? u() : unlistens.push(u)));
    void listen<{ session?: string | null }>("remote-terminal-opened", async (e) => {
      void refreshRemoteAccess();
      try {
        const { isPermissionGranted, sendNotification } = await import("@tauri-apps/plugin-notification");
        if (await isPermissionGranted())
          sendNotification({
            title: "Remote terminal opened",
            body: e.payload?.session ? `Your web app opened herdr session “${e.payload.session}” on this Mac.` : "Your web app opened a shell on this Mac.",
          });
      } catch {
        /* notifications unavailable */
      }
    }).then((u) => (cancelled ? u() : unlistens.push(u)));
    return () => {
      cancelled = true;
      unsubStore();
      for (const u of unlistens) u();
    };
  }, []);
}
