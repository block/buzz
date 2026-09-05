import * as React from "react";

import { useOpenMessageLink } from "@/app/navigation/useOpenMessageLink";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { listenForNavigationDeepLinks } from "@/shared/deep-link";

/**
 * Subscribe to `buzz://message` deep links emitted by the Tauri backend
 * and route them through the app's navigation helpers.
 *
 * Lives in a hook (not inline in `AppShell`) so it can be unit-tested
 * without the entire shell, and so the shell file stays under its line cap.
 *
 * Mirrors the cold-start race handling of the `connect` listener in
 * `App.tsx`: late-arriving payloads from a fresh launch are picked up the
 * first time the listener mounts. Routing matches the in-app buzz://
 * handler in `markdown.tsx`: resolve the target event's kind first, then
 * route forum posts and comments to `goForumPost` and everything else to
 * `goChannel`. `/channels/$channelId` cannot select a forum post, so
 * routing a forum target through it lands on the post list instead.
 */
export function useMessageDeepLinks(enabled = true) {
  const { goChannel } = useAppNavigation();
  const openMessageLink = useOpenMessageLink();

  React.useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    const unlistenPromise = listenForNavigationDeepLinks(
      async (payload) => {
        if (cancelled) return false;
        await goChannel(payload.channelId);
        return true;
      },
      async (payload) => {
        if (cancelled) return false;
        // Resolving `true` acks the link and drops it from the durable pending
        // queue, so the navigation has to have landed first — same contract the
        // channel listener above keeps by awaiting `goChannel`.
        await openMessageLink({
          channelId: payload.channelId,
          messageId: payload.messageId,
          threadRootId: payload.threadRootId,
        });
        return true;
      },
    );
    return () => {
      cancelled = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [enabled, goChannel, openMessageLink]);
}
