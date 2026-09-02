import { Hash } from "lucide-react";
import * as React from "react";

import type { Project } from "@/features/projects/projectModels";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

export function AddProjectChannelDialog({
  channels,
  isAdding,
  onAdd,
  onOpenChange,
  open,
  project,
}: {
  channels: Channel[];
  isAdding: boolean;
  onAdd: (channel: Channel) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  project: Project;
}) {
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (open) setErrorMessage(null);
  }, [open]);

  async function handleAdd(channel: Channel) {
    setErrorMessage(null);
    try {
      await onAdd(channel);
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : "Could not add the channel to this project.",
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isAdding) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-lg"
        contentClassName="max-h-96 overflow-y-auto pt-3"
        data-testid="add-project-channel-dialog"
        description={`Choose an existing channel to add to ${project.name}.`}
        title="Add existing channel"
      >
        <div className="space-y-2">
          {channels.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Every available channel is already in this project.
            </p>
          ) : (
            channels.map((channel) => (
              <Button
                className="h-auto w-full justify-start gap-3 px-3 py-2.5 text-left"
                data-testid={`add-existing-project-channel-${channel.id}`}
                disabled={isAdding}
                key={channel.id}
                onClick={() => void handleAdd(channel)}
                type="button"
                variant="outline"
              >
                <Hash className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0">
                  <span className="block truncate font-medium">
                    {channel.name}
                  </span>
                  {channel.description ? (
                    <span className="block truncate text-xs font-normal text-muted-foreground">
                      {channel.description}
                    </span>
                  ) : null}
                </span>
              </Button>
            ))
          )}
          {errorMessage ? (
            <p className="text-sm text-destructive" role="alert">
              {errorMessage}
            </p>
          ) : null}
        </div>
      </ChooserDialogContent>
    </Dialog>
  );
}
