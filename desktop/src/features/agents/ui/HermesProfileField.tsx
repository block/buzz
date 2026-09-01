import * as React from "react";

import { useHermesProfilesQuery } from "@/features/agents/useHermesProfiles";
import { Button } from "@/shared/ui/button";

export function HermesProfileField({
  disabled,
  onValueChange,
  value,
}: {
  disabled: boolean;
  onValueChange: (profileName: string) => void;
  value: string;
}) {
  const query = useHermesProfilesQuery({ enabled: true });
  const profiles = query.data?.profiles ?? [];
  const available = React.useMemo(
    () => profiles.filter((profile) => !profile.gatewayRunning),
    [profiles],
  );

  React.useEffect(() => {
    if (!value && available.length > 0 && !query.isLoading && !query.isError) {
      onValueChange(available[0].name);
    }
  }, [available, onValueChange, query.isError, query.isLoading, value]);

  return (
    <div className="space-y-1.5" data-testid="hermes-profile-field">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="hermes-profile"
      >
        Hermes profile on this computer
      </label>
      {query.isError ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          <p>
            {query.error instanceof Error
              ? query.error.message
              : "Could not discover Hermes profiles."}
          </p>
          <Button
            className="mt-2"
            onClick={() => void query.refetch()}
            size="sm"
            type="button"
            variant="outline"
          >
            Try again
          </Button>
        </div>
      ) : (
        <select
          className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="hermes-profile-select"
          disabled={disabled || query.isLoading || available.length === 0}
          id="hermes-profile"
          onChange={(event) => onValueChange(event.target.value)}
          value={value}
        >
          {query.isLoading ? (
            <option value="">Discovering profiles…</option>
          ) : null}
          {!query.isLoading && profiles.length === 0 ? (
            <option value="">No Hermes profiles found</option>
          ) : null}
          {profiles.map((profile) => (
            <option
              disabled={profile.gatewayRunning}
              key={profile.name}
              value={profile.name}
            >
              {profile.displayName || profile.name}
              {profile.model ? ` · ${profile.model}` : ""}
              {profile.gatewayRunning ? " · in use by gateway" : ""}
            </option>
          ))}
        </select>
      )}
      <p className="text-xs leading-relaxed text-muted-foreground">
        The profile binding belongs to this local managed instance. Buzz keeps
        the agent definition portable and never publishes the Hermes profile
        path.
      </p>
      {!query.isLoading && !query.isError && available.length === 0 ? (
        <p className="text-xs text-warning">
          Every discovered profile is currently in use by a Hermes gateway.
        </p>
      ) : null}
    </div>
  );
}
