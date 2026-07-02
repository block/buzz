import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  listenForChatDeepLinks,
  listenForMessageDeepLinks,
} from "@/shared/deep-link";

/**
 * Subscribe to routed Buzz deep links emitted by the Tauri backend
 * and route them through the app's navigation helpers.
 *
 * Lives in a hook (not inline in `AppShell`) so it can be unit-tested
 * without the entire shell, and so the shell file stays under its line cap.
 *
 * Mirrors the cold-start race handling of the `connect` listener in
 * `App.tsx`: late-arriving payloads from a fresh launch are picked up the
 * first time the listener mounts. Message routing matches the in-app
 * buzz:// handler in `markdown.tsx`: use `goChannel` with `messageId` and
 * let the channel route's existing scroll-into-view + getEventById backfill
 * resolve the target. Chat routing uses the chats route directly.
 */
export function useMessageDeepLinks() {
  const { goChannel, goChat } = useAppNavigation();

  React.useEffect(() => {
    let cancelled = false;
    const unlistenMessagePromise = listenForMessageDeepLinks((payload) => {
      if (cancelled) return;
      void goChannel(payload.channelId, {
        messageId: payload.messageId,
        threadRootId: payload.threadRootId,
      });
    });
    const unlistenChatPromise = listenForChatDeepLinks((payload) => {
      if (cancelled) return;
      void goChat(payload.chatId);
    });
    return () => {
      cancelled = true;
      void unlistenMessagePromise.then((fn) => fn());
      void unlistenChatPromise.then((fn) => fn());
    };
  }, [goChannel, goChat]);
}
