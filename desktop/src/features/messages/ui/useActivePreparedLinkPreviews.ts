import * as React from "react";
import type { PreparedBackgroundLinkPreviews } from "@/features/messages/lib/linkPreviewPreparationStore";

export function useActivePreparedLinkPreviews() {
  const preparations = React.useRef(new Set<PreparedBackgroundLinkPreviews>());
  React.useEffect(() => {
    const active = preparations.current;
    return () => {
      for (const preparation of active) preparation.cancel();
      active.clear();
    };
  }, []);
  return preparations.current;
}
