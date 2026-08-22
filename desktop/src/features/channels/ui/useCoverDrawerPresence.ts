import * as React from "react";

import { subscribeToCoverDrawerCloseRequest } from "@/features/channels/coverDrawerCloseRequest";

/** Keeps the covered channel inert and owns external dismissal while open. */
export function useCoverDrawerPresence(open: boolean, onClose: () => void) {
  const [present, setPresent] = React.useState(false);

  React.useEffect(() => {
    if (open) setPresent(true);
  }, [open]);

  React.useEffect(() => {
    if (!open) return;
    return subscribeToCoverDrawerCloseRequest(onClose);
  }, [onClose, open]);

  const markExitComplete = React.useCallback(() => setPresent(false), []);
  return {
    channelIsCovered: open || present,
    markExitComplete,
  };
}
