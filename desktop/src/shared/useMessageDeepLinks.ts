import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  listenForAgentConversationDeepLinks,
  listenForMessageDeepLinks,
} from "@/shared/deep-link";

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
 * handler in `markdown.tsx`: always `goChannel` with `messageId` and let
 * the channel route's existing scroll-into-view + getEventById backfill
 * resolve the target (works for both stream replies and forum threads).
 */
export function useMessageDeepLinks() {
  const { goChannel } = useAppNavigation();

  React.useEffect(() => {
    let cancelled = false;
    const messageUnlistenPromise = listenForMessageDeepLinks((payload) => {
      if (cancelled) return;
      void goChannel(payload.channelId, {
        messageId: payload.messageId,
        threadRootId: payload.threadRootId,
      });
    });
    const agentConversationUnlistenPromise =
      listenForAgentConversationDeepLinks((payload) => {
        if (cancelled) return;
        void goChannel(payload.channelId, {
          taskReplyId: payload.agentReplyId,
        });
      });

    return () => {
      cancelled = true;
      void messageUnlistenPromise.then((fn) => fn());
      void agentConversationUnlistenPromise.then((fn) => fn());
    };
  }, [goChannel]);
}
