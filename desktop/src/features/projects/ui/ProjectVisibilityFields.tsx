import { Check, Globe2, LockKeyhole, RefreshCw } from "lucide-react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

export type ProjectVisibility = "public" | "private";

type ProjectVisibilityFieldsProps = {
  channelId: string;
  channels: Channel[];
  channelsError: string | null;
  channelsLoading: boolean;
  disabled?: boolean;
  idPrefix: string;
  onChannelChange: (channelId: string) => void;
  onRetryChannels: () => void;
  onVisibilityChange: (visibility: ProjectVisibility) => void;
  visibility: ProjectVisibility;
};

export function eligibleProjectChannels(channels: Channel[] | undefined) {
  return (channels ?? []).filter(
    (channel) => channel.channelType !== "dm" && channel.isMember,
  );
}

const VISIBILITY_OPTIONS = [
  {
    description: "Anyone in the workspace can find and clone",
    icon: Globe2,
    label: "Public",
    value: "public",
  },
  {
    description: "Only members of one channel",
    icon: LockKeyhole,
    label: "Private",
    value: "private",
  },
] as const;

/** Shared, accessible repository visibility fields for create and edit flows. */
export function ProjectVisibilityFields({
  channelId,
  channels,
  channelsError,
  channelsLoading,
  disabled = false,
  idPrefix,
  onChannelChange,
  onRetryChannels,
  onVisibilityChange,
  visibility,
}: ProjectVisibilityFieldsProps) {
  const selectedChannel = channels.find((channel) => channel.id === channelId);
  const channelSelectId = `${idPrefix}-channel`;
  const channelHelpId = `${idPrefix}-channel-help`;

  return (
    <fieldset className="space-y-3" disabled={disabled}>
      <legend className="text-sm font-medium text-foreground">
        Visibility
      </legend>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {VISIBILITY_OPTIONS.map((option) => {
          const Icon = option.icon;
          const checked = visibility === option.value;
          return (
            <label className="cursor-pointer" key={option.value}>
              <input
                checked={checked}
                className="peer sr-only"
                data-testid={`${idPrefix}-visibility-${option.value}`}
                name={`${idPrefix}-visibility`}
                onChange={() => onVisibilityChange(option.value)}
                type="radio"
                value={option.value}
              />
              <span
                className={cn(
                  "flex min-h-24 items-start gap-3 rounded-xl border bg-muted/25 p-3 text-left transition-colors",
                  "border-input hover:border-muted-foreground/40 hover:bg-muted/40",
                  "peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2",
                  checked && "border-primary/50 bg-primary/5",
                )}
              >
                <span
                  className={cn(
                    "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground",
                    checked && "bg-primary/10 text-primary",
                  )}
                >
                  <Icon aria-hidden="true" className="h-4 w-4" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5 text-sm font-medium text-foreground">
                    {option.label}
                    {checked ? (
                      <Check
                        aria-hidden="true"
                        className="h-3.5 w-3.5 text-primary"
                      />
                    ) : null}
                  </span>
                  <span className="mt-1 block text-xs leading-5 text-muted-foreground">
                    {option.description}
                  </span>
                </span>
              </span>
            </label>
          );
        })}
      </div>

      {visibility === "private" ? (
        <div className="rounded-xl border border-input bg-muted/25 p-3">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor={channelSelectId}
          >
            Access channel
          </label>
          <select
            aria-describedby={channelHelpId}
            className="mt-2 min-h-10 w-full rounded-lg border border-input bg-background px-3 text-sm text-foreground outline-hidden focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
            data-testid={channelSelectId}
            disabled={
              disabled ||
              channelsLoading ||
              Boolean(channelsError) ||
              channels.length === 0
            }
            id={channelSelectId}
            onChange={(event) => onChannelChange(event.target.value)}
            value={channelId}
          >
            <option value="">
              {channelsLoading
                ? "Loading joined channels…"
                : channelsError
                  ? "Joined channels unavailable"
                  : channels.length === 0
                    ? "No joined channels available"
                    : "Choose a joined channel…"}
            </option>
            {channels.map((channel) => (
              <option key={channel.id} value={channel.id}>
                #{channel.name}
              </option>
            ))}
          </select>

          {channelsError ? (
            <div
              className="mt-2 flex items-start justify-between gap-3 text-xs text-destructive"
              role="alert"
            >
              <span>Couldn’t load your joined channels. {channelsError}</span>
              <Button
                className="h-7 shrink-0 gap-1 px-2"
                onClick={onRetryChannels}
                size="xs"
                type="button"
                variant="outline"
              >
                <RefreshCw aria-hidden="true" className="h-3 w-3" />
                Retry
              </Button>
            </div>
          ) : channelsLoading ? (
            <p className="mt-2 text-xs text-muted-foreground" role="status">
              Loading channels you currently belong to…
            </p>
          ) : channels.length === 0 ? (
            <p className="mt-2 text-xs text-muted-foreground">
              Join a channel to create a private project
            </p>
          ) : (
            <p
              className="mt-2 text-xs leading-5 text-muted-foreground"
              id={channelHelpId}
            >
              {selectedChannel
                ? `Current members of #${selectedChannel.name} get access immediately; removed members lose access on their next request.`
                : "Choose the channel whose current members should be able to discover and use this repository."}
            </p>
          )}
        </div>
      ) : null}
    </fieldset>
  );
}
