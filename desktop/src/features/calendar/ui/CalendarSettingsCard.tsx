import { CalendarDays, Check, ExternalLink, Plug, Unplug } from "lucide-react";

import {
  useGoogleCalendarConnectionMutations,
  useGoogleCalendarStatusQuery,
} from "@/features/calendar/hooks";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

function errorMessage(error: unknown): string | null {
  if (!error) return null;
  return error instanceof Error ? error.message : String(error);
}

export function ConnectionsSettingsCard() {
  const statusQuery = useGoogleCalendarStatusQuery();
  const { connect, disconnect } = useGoogleCalendarConnectionMutations();
  const status = statusQuery.data;
  const isBusy = connect.isPending || disconnect.isPending;
  const isLoading = statusQuery.isLoading;
  const mutationError =
    errorMessage(connect.error) ?? errorMessage(disconnect.error);
  const statusError = errorMessage(statusQuery.error);
  const description = isLoading
    ? "Checking Google Calendar connection status."
    : status?.connected
      ? "Connected with read-only event access. Meeting details stay on this device."
      : status?.configured
        ? "Connect to read upcoming events and video meeting links."
        : "Google Calendar is unavailable until this build includes a Google OAuth client ID.";

  return (
    <section className="min-w-0" data-testid="settings-connections">
      <SettingsSectionHeader
        title="Connections"
        description="Connect apps on this device so Buzz can bring their context into your workspace."
      />

      <div className="flex flex-col gap-4">
        <SettingsOptionGroup>
          <SettingsOptionRow>
            <div className="min-w-0">
              <span className="flex items-center gap-2 text-sm font-medium">
                <CalendarDays className="h-4 w-4 text-muted-foreground" />
                Google Calendar
              </span>
              <p className="text-sm font-normal text-muted-foreground">
                {description}
              </p>
            </div>
            {status?.connected ? (
              <Button
                disabled={isBusy}
                onClick={() => disconnect.mutate()}
                size="sm"
                type="button"
                variant="outline"
              >
                <Unplug />
                Disconnect
              </Button>
            ) : (
              <Button
                disabled={isLoading || isBusy || !status?.configured}
                onClick={() => connect.mutate()}
                size="sm"
                type="button"
              >
                <Plug />
                Connect
              </Button>
            )}
          </SettingsOptionRow>
        </SettingsOptionGroup>

        {status?.connected ? (
          <div className="flex items-center gap-2 rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-sm text-primary">
            <Check className="h-4 w-4 shrink-0" />
            <span className="min-w-0">
              Google Calendar is ready. The sidebar will show current and
              upcoming meetings.
            </span>
          </div>
        ) : null}

        {!isLoading && !status?.configured ? (
          <div className="rounded-lg border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground">
            Connect is unavailable because this build is missing its Google
            OAuth client ID. End users do not create this themselves; local and
            staging builds need{" "}
            <code className="rounded bg-muted px-1 py-0.5 text-xs">
              BUZZ_GOOGLE_CALENDAR_CLIENT_ID
            </code>
            . If the OAuth app has a client secret, also provide{" "}
            <code className="rounded bg-muted px-1 py-0.5 text-xs">
              BUZZ_GOOGLE_CALENDAR_CLIENT_SECRET
            </code>
            .
          </div>
        ) : null}

        {!status?.connected && status?.configured ? (
          <div className="rounded-lg border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground">
            Connect opens Google OAuth in your browser. After you approve
            read-only calendar access, Buzz stores the refresh token in the
            system keychain.
          </div>
        ) : null}

        {statusError || mutationError ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {mutationError ?? statusError}
          </div>
        ) : null}

        <a
          className="inline-flex w-fit items-center gap-1.5 text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
          href="https://developers.google.com/identity/protocols/oauth2/native-app"
          rel="noreferrer"
          target="_blank"
        >
          Google OAuth desktop app setup
          <ExternalLink className="h-3.5 w-3.5" />
        </a>
      </div>
    </section>
  );
}

export const CalendarSettingsCard = ConnectionsSettingsCard;
