import * as React from "react";

import {
  getAvatarScaleServerSnapshot,
  getAvatarScaleSnapshot,
  subscribeAvatarScale,
} from "@/shared/lib/avatarScale";

/** React hook for the message avatar scale factor. */
export function useAvatarScale(): number {
  return React.useSyncExternalStore(
    subscribeAvatarScale,
    getAvatarScaleSnapshot,
    getAvatarScaleServerSnapshot,
  );
}
