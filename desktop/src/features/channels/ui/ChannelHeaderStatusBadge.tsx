import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { EphemeralChannelBadge } from "@/features/channels/ui/EphemeralChannelBadge";

type ChannelHeaderStatusBadgeProps = {
  ephemeralDisplay: EphemeralChannelDisplay | null;
};

export function ChannelHeaderStatusBadge({
  ephemeralDisplay,
}: ChannelHeaderStatusBadgeProps) {
  return ephemeralDisplay ? (
    <EphemeralChannelBadge
      display={ephemeralDisplay}
      testId="chat-ephemeral-badge"
      variant="header"
    />
  ) : null;
}
