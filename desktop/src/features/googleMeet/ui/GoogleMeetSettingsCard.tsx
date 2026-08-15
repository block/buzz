import { LoaderCircle, Video } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import {
  useCancelGoogleMeetConnectMutation,
  useConnectGoogleMeetMutation,
  useDisconnectGoogleMeetMutation,
  useGoogleMeetConnectionQuery,
} from "../hooks";

function connectErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.toLowerCase().includes("not configured for this build")) {
    return "Google Meet isn't set up for this build of Buzz yet.";
  }
  if (message.toLowerCase().includes("canceled")) {
    return "Sign-in was canceled.";
  }
  return message || "Couldn't connect your Google account. Try again.";
}

/**
 * Per-device Google account connection for the Google Meet integration
 * (see `features/googleMeet`). Each teammate connects their own account —
 * there's no shared/org-wide credential — so "start a Google Meet" in a
 * channel works from whichever account is connected on that device.
 */
export function GoogleMeetSettingsCard() {
  const connectionQuery = useGoogleMeetConnectionQuery();
  const connectMutation = useConnectGoogleMeetMutation();
  const cancelConnectMutation = useCancelGoogleMeetConnectMutation();
  const disconnectMutation = useDisconnectGoogleMeetMutation();

  async function handleConnect() {
    try {
      await connectMutation.mutateAsync();
      toast.success("Google account connected");
    } catch (error) {
      console.error("Failed to connect Google account:", error);
      toast.error(connectErrorMessage(error));
    }
  }

  async function handleDisconnect() {
    try {
      await disconnectMutation.mutateAsync();
      toast.success("Google account disconnected");
    } catch (error) {
      console.error("Failed to disconnect Google account:", error);
      toast.error("Couldn't disconnect. Try again.");
    }
  }

  const isConnected = connectionQuery.data === true;
  const isConnecting = connectMutation.isPending;

  return (
    <section className="min-w-0" data-testid="settings-google-meet">
      <SettingsSectionHeader
        title="Google Meet"
        description={
          <>
            Connect your Google account to start instant Google Meet calls from
            a channel or DM — an alternative to Huddle backed by Google's own
            call infrastructure.
          </>
        }
      />

      <SettingsOptionGroup data-testid="google-meet-connection-card">
        <SettingsOptionRow>
          <div className="flex min-w-0 items-center gap-3">
            <span
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-background"
              aria-hidden="true"
            >
              <Video className="h-4 w-4 text-muted-foreground" />
            </span>
            <div className="min-w-0">
              <p className="font-medium">
                {isConnected ? "Google account connected" : "Not connected"}
              </p>
              <p className="text-xs text-muted-foreground/80">
                {isConnected
                  ? "You can start Google Meet calls from any channel or DM."
                  : "Connect your Google account to start Google Meet calls."}
              </p>
            </div>
          </div>

          {isConnecting ? (
            <Button
              data-testid="cancel-google-meet-connect"
              onClick={() => void cancelConnectMutation.mutate()}
              size="sm"
              type="button"
              variant="outline"
            >
              <LoaderCircle className="animate-spin" />
              Cancel
            </Button>
          ) : isConnected ? (
            <Button
              data-testid="disconnect-google-meet"
              disabled={disconnectMutation.isPending}
              onClick={() => void handleDisconnect()}
              size="sm"
              type="button"
              variant="outline"
            >
              Disconnect
            </Button>
          ) : (
            <Button
              data-testid="connect-google-meet"
              disabled={connectionQuery.isPending}
              onClick={() => void handleConnect()}
              size="sm"
              type="button"
            >
              Connect Google account
            </Button>
          )}
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
