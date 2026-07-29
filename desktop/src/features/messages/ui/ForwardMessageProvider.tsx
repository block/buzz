import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { ForwardMessageDialog } from "./ForwardMessageDialog";

export type ForwardMessageTarget = {
  message: TimelineMessage;
  /** Channel the message is being forwarded FROM (the surface it lives on). */
  channelId: string;
};

type ForwardMessageContextValue = {
  openForwardDialog: (target: ForwardMessageTarget) => void;
};

const ForwardMessageContext = React.createContext<ForwardMessageContextValue>({
  openForwardDialog: () => {},
});

export function useForwardMessage() {
  return React.useContext(ForwardMessageContext);
}

/**
 * Hosts the "Forward message…" dialog and exposes `openForwardDialog` to any
 * message surface below it (timeline, thread panel, DMs). Mirrors
 * `RemindMeLaterProvider` structurally.
 */
export function ForwardMessageProvider({
  pubkey,
  children,
}: {
  pubkey?: string;
  children: React.ReactNode;
}) {
  const [open, setOpen] = React.useState(false);
  const [target, setTarget] = React.useState<ForwardMessageTarget | null>(null);

  const openForwardDialog = React.useCallback((t: ForwardMessageTarget) => {
    setTarget(t);
    setOpen(true);
  }, []);

  const contextValue = React.useMemo(
    () => ({ openForwardDialog }),
    [openForwardDialog],
  );

  return (
    <ForwardMessageContext.Provider value={contextValue}>
      {children}
      <ForwardMessageDialog
        currentPubkey={pubkey}
        onOpenChange={setOpen}
        open={open}
        target={target}
      />
    </ForwardMessageContext.Provider>
  );
}
