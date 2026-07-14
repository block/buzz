import * as React from "react";
import { toast } from "sonner";

import { subscribeToRelayNotices } from "@/shared/api/relayNotices";

export function useRelayNoticeToasts(): void {
  React.useEffect(() => {
    return subscribeToRelayNotices((message) => {
      toast.warning(message);
    });
  }, []);
}
