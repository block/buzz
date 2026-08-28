import { Video } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { buildChannelMeetingSearch } from "@/features/meetings/ui/meetingsDeepLink";
import { useMeetingsCapability } from "@/features/meetings/useMeetingsCapability";
import { useFeatureEnabled } from "@/shared/features";
import { Button } from "@/shared/ui/button";

type StartMeetingButtonProps = {
  channelId: string;
  channelName: string;
};

/**
 * Channel-header shortcut into the Meetings tab, pre-filled with a room name
 * derived from the channel. Renders only when the active community relay
 * advertises the Meetings capability.
 */
export function StartMeetingButton({
  channelId,
  channelName,
}: StartMeetingButtonProps) {
  const meetingsEnabled = useFeatureEnabled("meetings");
  const { capability } = useMeetingsCapability();
  const { goMeetings } = useAppNavigation();

  if (!meetingsEnabled || capability === null) {
    return null;
  }

  return (
    <Button
      aria-label="Start meeting"
      onClick={() => {
        const search = buildChannelMeetingSearch({ channelId, channelName });
        void goMeetings({ action: search.action, room: search.room });
      }}
      size="icon"
      title="Start meeting"
      type="button"
      variant="outline"
    >
      <Video />
    </Button>
  );
}
