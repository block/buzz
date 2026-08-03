import * as React from "react";

import {
  getTextScaleServerSnapshot,
  getTextScaleSnapshot,
  subscribeTextScale,
} from "@/shared/lib/textScale";

/** React hook for the current text / UI scale factor. */
export function useTextScale(): number {
  return React.useSyncExternalStore(
    subscribeTextScale,
    getTextScaleSnapshot,
    getTextScaleServerSnapshot,
  );
}
