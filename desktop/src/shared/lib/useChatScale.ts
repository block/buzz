import * as React from "react";

import {
  getChatScaleServerSnapshot,
  getChatScaleSnapshot,
  subscribeChatScale,
} from "@/shared/lib/chatScale";

/** React hook for the chat / message content scale factor. */
export function useChatScale(): number {
  return React.useSyncExternalStore(
    subscribeChatScale,
    getChatScaleSnapshot,
    getChatScaleServerSnapshot,
  );
}
