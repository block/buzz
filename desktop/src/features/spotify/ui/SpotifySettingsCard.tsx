import { Music2, Plug, Unplug } from "lucide-react";

import {
  useSpotifyConnectionMutations,
  useSpotifyStatusQuery,
} from "@/features/spotify/hooks";
import { Button } from "@/shared/ui/button";
import {
  SettingsOptionGroup,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { Spinner } from "@/shared/ui/spinner";

function errorMessage(error: unknown): string | null {
  if (!error) return null;
  return error instanceof Error ? error.message : String(error);
}

export function SpotifySettingsCard() {
  const statusQuery = useSpotifyStatusQuery();
  const status = statusQuery.data;
  const connected = Boolean(status?.connected);
  const { connect, disconnect } = useSpotifyConnectionMutations();
  const isBusy = connect.isPending || disconnect.isPending;
  const isLoading = statusQuery.isLoading;
  const mutationError =
    errorMessage(connect.error) ?? errorMessage(disconnect.error);
  const statusError = errorMessage(statusQuery.error);

  return (
    <div className="flex flex-col gap-4" data-testid="settings-spotify">
      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <span className="flex items-center gap-2 text-sm font-medium">
              <Music2 className="h-4 w-4 text-muted-foreground" />
              Spotify
            </span>
          </div>
          {connected ? (
            <Button
              disabled={isBusy}
              onClick={() => disconnect.mutate()}
              size="sm"
              type="button"
              variant="outline"
            >
              {disconnect.isPending ? (
                <Spinner className="h-4 w-4" />
              ) : (
                <Unplug />
              )}
              Disconnect
            </Button>
          ) : (
            <Button
              disabled={isLoading || isBusy || !status?.configured}
              onClick={() => connect.mutate()}
              size="sm"
              type="button"
            >
              {isLoading || connect.isPending ? (
                <Spinner className="h-4 w-4" />
              ) : (
                <Plug />
              )}
              Connect
            </Button>
          )}
        </SettingsOptionRow>
      </SettingsOptionGroup>

      {statusError || mutationError ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {mutationError ?? statusError}
        </div>
      ) : null}
    </div>
  );
}
