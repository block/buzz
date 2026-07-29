import type * as React from "react";

import { ForwardMessageProvider } from "@/features/messages/ui/ForwardMessageProvider";
import { RemindMeLaterProvider } from "@/features/reminders/ui/RemindMeLaterProvider";

/**
 * Composes the message-action dialog hosts (remind-me-later, forward) that
 * every message surface under the app shell shares, keeping `AppShell`'s JSX
 * nesting flat as hosts are added.
 */
export function MessageActionProviders({
  pubkey,
  children,
}: {
  pubkey?: string;
  children: React.ReactNode;
}) {
  return (
    <RemindMeLaterProvider pubkey={pubkey}>
      <ForwardMessageProvider pubkey={pubkey}>
        {children}
      </ForwardMessageProvider>
    </RemindMeLaterProvider>
  );
}
