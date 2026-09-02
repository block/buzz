import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

export function LinkProjectChannelDialog({
  channels,
  isPending,
  onLink,
  onOpenChange,
  open,
  projectName,
}: {
  channels: Channel[];
  isPending: boolean;
  onLink: (channelId: string) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  projectName: string;
}) {
  const [selectedChannelId, setSelectedChannelId] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const selectRef = React.useRef<HTMLSelectElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setSelectedChannelId("");
    setError(null);
    const timer = globalThis.setTimeout(() => selectRef.current?.focus(), 50);
    return () => globalThis.clearTimeout(timer);
  }, [open]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedChannelId) return;
    setError(null);
    try {
      await onLink(selectedChannelId);
      onOpenChange(false);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not link the channel.",
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isPending) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-md"
        contentClassName="pt-3"
        data-testid="link-project-channel-dialog"
        footer={
          <Button
            data-testid="link-project-channel-submit"
            disabled={isPending || !selectedChannelId}
            form="link-project-channel-form"
            type="submit"
          >
            {isPending ? "Linking…" : "Link channel"}
          </Button>
        }
        footerClassName="justify-end"
        headerSubtitle={`Choose an existing channel to add to ${projectName}.`}
        title="Link a channel"
      >
        <form
          id="link-project-channel-form"
          onSubmit={(event) => void handleSubmit(event)}
        >
          {channels.length > 0 ? (
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="link-project-channel-select"
              >
                Channel
              </label>
              <select
                className="h-10 w-full rounded-lg border border-input bg-background px-3 text-sm outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                data-testid="link-project-channel-select"
                disabled={isPending}
                id="link-project-channel-select"
                onChange={(event) => {
                  setSelectedChannelId(event.target.value);
                  setError(null);
                }}
                ref={selectRef}
                required
                value={selectedChannelId}
              >
                <option value="">Select a channel</option>
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    #{channel.name}
                  </option>
                ))}
              </select>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              Every active channel you belong to is already linked.
            </p>
          )}
          {error ? (
            <p className="mt-3 text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
