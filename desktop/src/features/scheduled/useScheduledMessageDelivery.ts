import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { sendChannelMessage } from "@/shared/api/tauri";
import {
  reenqueueScheduledMessage,
  takeDueScheduledMessages,
} from "@/shared/api/scheduledMessages";
import { relayClient } from "@/shared/api/relayClient";
import { invalidateScheduledMessages } from "@/features/scheduled/useScheduledMessages";

/**
 * How often the delivery loop sweeps the queue. Coarse enough to be cheap
 * (a filesystem read per tick), fine enough to deliver "in a few minutes"
 * schedules within a reasonable window.
 */
export const SCHEDULED_DELIVERY_SWEEP_INTERVAL_MS = 15_000;

/**
 * Delivers due scheduled messages while the app is open.
 *
 * The queue lives in a shared local file (`<app-data>/scheduled/
 * scheduled-messages.json`) that the composer writes and the CLI can also
 * consume. This hook owns the *delivery* side for the desktop: on every
 * sweep it atomically takes due entries (Rust `scheduled_take_due`), replays
 * them through the normal REST send path (mentions and thread refs are
 * resolved at delivery time), and re-enqueues anything that fails
 * transiently so a later sweep retries it.
 */
export function useScheduledMessageDelivery() {
  const sweepingRef = React.useRef(false);
  const queryClient = useQueryClient();

  const sweep = React.useCallback(async () => {
    // Never overlap sweeps — a slow REST publish must not race a second
    // `take_due` on the same shared queue.
    if (sweepingRef.current) return;
    sweepingRef.current = true;
    try {
      const due = await takeDueScheduledMessages();
      if (due.length === 0) return;

      let deliveredCount = 0;
      for (const message of due) {
        try {
          await sendChannelMessage(
            message.channelId,
            message.content,
            message.replyTo ?? null,
            undefined,
            message.mentions,
          );
          deliveredCount += 1;
        } catch {
          // Transient (network / relay overload): put it back for a later
          // sweep instead of dropping it.
          try {
            await reenqueueScheduledMessage(message);
          } catch {
            // Store is unavailable too — the entry is lost either way; the
            // message content stays visible in the CLI queue if it was
            // re-saved, otherwise nothing more we can do here.
          }
        }
      }
      if (deliveredCount > 0) {
        await invalidateScheduledMessages(queryClient);
      }
    } catch {
      // The store itself was unavailable (e.g. app-data dir locked).
      // Retry on the next sweep.
    } finally {
      sweepingRef.current = false;
    }
  }, [queryClient]);

  React.useEffect(() => {
    // First sweep shortly after mount (after identity/relay settle), then on
    // a fixed cadence. Skip while the relay is not connected — REST delivery
    // would fail anyway and the re-enqueue churn buys nothing.
    const initialTimer = window.setTimeout(() => {
      if (relayClient.getConnectionState() !== "connected") return;
      void sweep();
    }, 5_000);

    const interval = window.setInterval(() => {
      if (relayClient.getConnectionState() !== "connected") return;
      void sweep();
    }, SCHEDULED_DELIVERY_SWEEP_INTERVAL_MS);

    return () => {
      window.clearTimeout(initialTimer);
      window.clearInterval(interval);
    };
  }, [sweep]);
}
