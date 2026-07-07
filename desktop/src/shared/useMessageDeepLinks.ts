import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { resolveChatOpenDestination } from "@/features/chats/lib/chatOpenDestination";
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
 * resolve the target. Chat routing prefers the chats route, then falls back
 * to the underlying private channel when chat metadata is unavailable.
 */
export function useMessageDeepLinks() {
  const { goChannel, goChat } = useAppNavigation();

  React.useEffect(() => {
    let cancelled = false;
    const openChatOrChannel = async (channelId: string) => {
      const destination = await resolveChatOpenDestination(channelId);
      if (cancelled) return;
      if (destination.kind === "chat") {
        void goChat(destination.chatId);
      } else {
        void goChannel(destination.channelId);
      }
    };
    const unlistenMessagePromise = listenForMessageDeepLinks((payload) => {
      if (cancelled) return;
      void resolveChatOpenDestination(payload.channelId).then((destination) => {
        if (cancelled) return;
        if (destination.kind === "chat") {
          void goChat(destination.chatId);
          return;
        }
        void goChannel(destination.channelId, {
          messageId: payload.messageId,
          threadRootId: payload.threadRootId,
        });
      });
    });
    const unlistenChatPromise = listenForChatDeepLinks((payload) => {
      if (cancelled) return;
      void openChatOrChannel(payload.chatId);
    });
    return () => {
      cancelled = true;
      void unlistenMessagePromise.then((fn) => fn());
      void unlistenChatPromise.then((fn) => fn());
    };
  }, [goChannel, goChat]);
}
