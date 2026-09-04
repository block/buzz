import * as React from "react";

import {
  consumePendingOpenEditAgent,
  type EditAgentFocusTarget,
  subscribeOpenEditAgent,
} from "@/features/agents/openEditAgentEvent";

export function useProfileEditAgentRequest(pubkey: string | null | undefined) {
  const [open, setOpen] = React.useState(false);
  const [focus, setFocus] = React.useState<EditAgentFocusTarget | undefined>(
    undefined,
  );

  React.useEffect(() => {
    if (!pubkey) {
      return;
    }

    const pending = consumePendingOpenEditAgent(pubkey);
    if (pending !== false) {
      setFocus(pending === true ? undefined : pending);
      setOpen(true);
    }

    return subscribeOpenEditAgent(pubkey, (nextFocus) => {
      setFocus(nextFocus);
      setOpen(true);
    });
  }, [pubkey]);

  return { focus, open, setFocus, setOpen };
}
