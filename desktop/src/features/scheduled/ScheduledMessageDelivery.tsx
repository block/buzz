import { useScheduledMessageDelivery } from "@/features/scheduled/useScheduledMessageDelivery";

/**
 * App-wide mount point for the scheduled-message delivery loop. Renders
 * nothing; the delivery happens in the hook while the app is open.
 */
export function ScheduledMessageDelivery() {
  useScheduledMessageDelivery();
  return null;
}
