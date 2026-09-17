import * as React from "react";

import {
  acquireNativeWebviewModalPark,
  releaseNativeWebviewModalPark,
} from "@/shared/lib/nativeWebviewModalPark";

/**
 * While mounted, parks main-window native WKWebViews so React overlays
 * (Dialog / Sheet / AlertDialog / custom modals) stay interactive.
 * Mount from portal content that only exists while the overlay is open.
 */
export function ParkNativeWebviewsWhileMounted(): null {
  React.useEffect(() => {
    acquireNativeWebviewModalPark();
    return () => {
      releaseNativeWebviewModalPark();
    };
  }, []);
  return null;
}
