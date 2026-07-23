import { Check, ChevronDown, Globe2, LockKeyhole } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useChannelsQuery } from "@/features/channels/hooks";
import type { Project } from "@/features/projects/hooks";
import { useUpdateProjectVisibilityMutation } from "@/features/projects/useCreateProject";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  eligibleProjectChannels,
  type ProjectVisibility,
} from "./ProjectVisibilityFields";

function visibilitySummary(project: Project, channelName: string | null) {
  if (project.visibility === "public") return "Public";
  return channelName ? `Private · #${channelName}` : "Private";
}

export function ProjectVisibilityControl({
  currentPubkey,
  project,
}: {
  currentPubkey: string | null;
  project: Project;
}) {
  const channelsQuery = useChannelsQuery();
  const mutation = useUpdateProjectVisibilityMutation();
  const channels = eligibleProjectChannels(channelsQuery.data);
  const canEdit = currentPubkey?.toLowerCase() === project.owner.toLowerCase();
  const boundChannel = channels.find(
    (channel) => channel.id === project.privateChannelId,
  );
  const summary = visibilitySummary(project, boundChannel?.name ?? null);
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [confirmPublicOpen, setConfirmPublicOpen] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const Icon = project.visibility === "private" ? LockKeyhole : Globe2;

  async function updateVisibility(
    visibility: ProjectVisibility,
    channelId?: string,
  ) {
    if (!canEdit) return;

    setErrorMessage(null);
    try {
      await mutation.mutateAsync({
        project,
        visibility,
        channelId: visibility === "private" ? channelId : undefined,
      });
      setConfirmPublicOpen(false);
      toast.success(
        visibility === "private"
          ? "Project access updated."
          : "Project is now public to workspace members.",
      );
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "Failed to update project visibility.";
      setErrorMessage(message);
      toast.error(message);
    }
  }

  if (!canEdit) {
    const status = (
      <span
        className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border px-2.5 text-xs text-muted-foreground"
        data-testid="project-visibility-status"
      >
        <Icon aria-hidden="true" className="h-3.5 w-3.5" />
        {summary}
      </span>
    );

    if (project.visibility === "private" && boundChannel) {
      return (
        <Tooltip>
          <TooltipTrigger asChild>{status}</TooltipTrigger>
          <TooltipContent>
            Only members of #{boundChannel.name} and the owner can access this
            project.
          </TooltipContent>
        </Tooltip>
      );
    }

    return status;
  }

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (mutation.isPending) return;
        setConfirmPublicOpen(nextOpen);
      }}
      open={confirmPublicOpen}
    >
      <DropdownMenu
        modal={false}
        onOpenChange={(nextOpen) => {
          if (mutation.isPending) return;
          setMenuOpen(nextOpen);
          if (nextOpen) setErrorMessage(null);
        }}
        open={menuOpen}
      >
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={`Configure project visibility. Current setting: ${summary}`}
            className="h-8 max-w-64 gap-1.5"
            data-testid="project-visibility-trigger"
            disabled={mutation.isPending}
            size="sm"
            variant="outline"
          >
            <Icon aria-hidden="true" className="h-3.5 w-3.5" />
            <span className="truncate">{summary}</span>
            <ChevronDown
              aria-hidden="true"
              className="h-3 w-3 text-muted-foreground"
            />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="w-72 max-w-[calc(100vw-2rem)]"
          data-testid="project-visibility-menu"
        >
          <DropdownMenuLabel className="text-xs text-muted-foreground">
            Repository access
          </DropdownMenuLabel>
          <DropdownMenuItem
            data-testid="project-visibility-public"
            onSelect={(event) => {
              if (project.visibility === "private") {
                event.preventDefault();
                setMenuOpen(false);
                setConfirmPublicOpen(true);
              }
            }}
          >
            <Globe2 aria-hidden="true" />
            <span className="flex-1">Public</span>
            {project.visibility === "public" ? (
              <Check aria-hidden="true" className="text-primary" />
            ) : null}
          </DropdownMenuItem>
          <DropdownMenuSub>
            <DropdownMenuSubTrigger data-testid="project-visibility-private">
              <LockKeyhole aria-hidden="true" />
              <span className="flex-1">Private</span>
              {project.visibility === "private" ? (
                <Check aria-hidden="true" className="text-primary" />
              ) : null}
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent
              className="w-72 max-w-[calc(100vw-2rem)]"
              data-testid="project-visibility-channel-menu"
            >
              <DropdownMenuLabel className="text-xs text-muted-foreground">
                Choose an access channel
              </DropdownMenuLabel>
              {channelsQuery.isLoading ? (
                <DropdownMenuItem disabled>
                  Loading joined channels…
                </DropdownMenuItem>
              ) : channelsQuery.isError ? (
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onSelect={(event) => {
                    event.preventDefault();
                    void channelsQuery.refetch();
                  }}
                >
                  Couldn’t load channels. Retry
                </DropdownMenuItem>
              ) : channels.length === 0 ? (
                <DropdownMenuItem disabled>
                  Join a channel to make this project private
                </DropdownMenuItem>
              ) : (
                channels.map((channel) => (
                  <DropdownMenuItem
                    data-testid={`project-visibility-channel-${channel.id}`}
                    key={channel.id}
                    onSelect={() => {
                      if (
                        project.visibility === "private" &&
                        project.privateChannelId === channel.id
                      ) {
                        return;
                      }
                      setMenuOpen(false);
                      void updateVisibility("private", channel.id);
                    }}
                  >
                    <span className="flex-1 truncate">#{channel.name}</span>
                    {project.visibility === "private" &&
                    project.privateChannelId === channel.id ? (
                      <Check aria-hidden="true" className="text-primary" />
                    ) : null}
                  </DropdownMenuItem>
                ))
              )}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        </DropdownMenuContent>
      </DropdownMenu>

      <AlertDialogContent data-testid="project-visibility-public-confirm">
        <AlertDialogHeader>
          <AlertDialogTitle>Make this project public?</AlertDialogTitle>
          <AlertDialogDescription>
            Anyone in the workspace will be able to find and clone it.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {errorMessage ? (
          <p
            className="rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive"
            data-testid="project-visibility-error"
            role="alert"
          >
            {errorMessage}
          </p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={mutation.isPending}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid="project-visibility-public-confirm-button"
              disabled={mutation.isPending}
              onClick={(event) => {
                event.preventDefault();
                void updateVisibility("public");
              }}
              type="button"
            >
              {mutation.isPending ? "Making public…" : "Make public"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
