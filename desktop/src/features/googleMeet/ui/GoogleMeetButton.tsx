import { LoaderCircle, Video } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useSendMessageMutation } from "@/features/messages/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  useConnectGoogleMeetMutation,
  useCreateInstantGoogleMeetMutation,
  useGoogleMeetConnectionQuery,
} from "../hooks";

type GoogleMeetButtonProps = {
  channel: Channel;
  className?: string;
  disabled?: boolean;
  renderMode?: "button" | "menu-item";
};

function formatGoogleMeetError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.toLowerCase().includes("not configured for this build")) {
    return "Google Meet isn't set up for this build of Buzz yet.";
  }
  if (message.toLowerCase().includes("canceled")) {
    return "Google sign-in was canceled.";
  }
  return message || "Couldn't start a Google Meet. Try again.";
}

/**
 * Alternative to Huddle for voice/video calls: creates an instant Google
 * Meet (via each user's own connected Google account) and posts the join
 * link as a normal channel message. Deliberately does not use Huddle's
 * relay-event presence mechanism — a Meet link needs no custom event kind,
 * it's just a message like any shared URL.
 */
export function GoogleMeetButton({
  channel,
  className,
  disabled,
  renderMode = "button",
}: GoogleMeetButtonProps) {
  const identityQuery = useIdentityQuery();
  const connectionQuery = useGoogleMeetConnectionQuery();
  const connectMutation = useConnectGoogleMeetMutation();
  const createMeetingMutation = useCreateInstantGoogleMeetMutation();
  const sendMessageMutation = useSendMessageMutation(
    channel,
    identityQuery.data,
  );
  const [isStarting, setIsStarting] = React.useState(false);

  async function handleStart() {
    if (isStarting) return;
    setIsStarting(true);
    try {
      if (!connectionQuery.data) {
        toast.info("Opening Google sign-in in your browser…");
        await connectMutation.mutateAsync();
      }

      const meeting = await createMeetingMutation.mutateAsync();
      await sendMessageMutation.mutateAsync({
        channelId: channel.id,
        content: `Started a Google Meet — join: ${meeting.meetingUri}`,
      });
      toast.success("Google Meet started");
    } catch (error) {
      console.error("Failed to start Google Meet:", error);
      toast.error(formatGoogleMeetError(error));
    } finally {
      setIsStarting(false);
    }
  }

  const isBusy = isStarting || connectionQuery.isPending;
  const icon = isStarting ? (
    <LoaderCircle className="animate-spin" />
  ) : (
    <Video />
  );

  if (renderMode === "menu-item") {
    return (
      <DropdownMenuItem
        className={className}
        data-testid="channel-start-google-meet-trigger"
        disabled={disabled || isBusy}
        onSelect={() => void handleStart()}
      >
        {icon}
        <span>Start Google Meet</span>
      </DropdownMenuItem>
    );
  }

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <span
          className="inline-flex"
          data-testid="channel-google-meet-tooltip-trigger"
        >
          <Button
            aria-label="Start Google Meet"
            className={cn(className)}
            data-testid="channel-start-google-meet-trigger"
            disabled={disabled || isBusy}
            onClick={() => void handleStart()}
            size="icon"
            type="button"
            variant="outline"
          >
            {icon}
          </Button>
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {connectionQuery.data
          ? "Start a Google Meet"
          : "Connect Google & start a Meet"}
      </TooltipContent>
    </Tooltip>
  );
}
