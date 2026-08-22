import { Bot, GitPullRequest, Link2, X } from "lucide-react";
import * as React from "react";

import {
  projectSelectionShareLinks,
  type ProjectSelectionAction,
  type ProjectSelectionItem,
  type ProjectSelectionPresentation,
} from "@/features/projects/lib/projectSelection";
import { useProjectSelection } from "@/features/projects/lib/useProjectSelection";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import { ProjectSelectionDiscussAction } from "./ProjectSelectionDiscussAction";
import { useProjectDiscussInChannel } from "./useProjectDiscussInChannel";

function selectionActionIcon(id: ProjectSelectionAction["id"]) {
  if (id === "chat-agent") return Bot;
  if (id === "create-review") return GitPullRequest;
  return Link2;
}

/** Inline actions for the current Projects selection. */
export function ProjectsSelectionCountMenu({
  onChatWithAgent,
  onCreatePullRequest,
  presentation,
  selectionItems,
}: {
  onChatWithAgent: (items: ProjectSelectionItem[]) => void;
  onCreatePullRequest?: () => void;
  presentation: ProjectSelectionPresentation;
  selectionItems: ProjectSelectionItem[];
}) {
  const selection = useProjectSelection();
  const openChannelWithDraft = useProjectDiscussInChannel(selectionItems);

  const discussInChannel = React.useCallback(
    (channelId: string) => {
      openChannelWithDraft(channelId);
      selection?.clear();
    },
    [openChannelWithDraft, selection],
  );

  const handleAction = React.useCallback(
    (actionId: ProjectSelectionAction["id"]) => {
      if (actionId === "chat-agent") {
        onChatWithAgent(selectionItems);
        selection?.clear();
        return;
      }
      if (actionId === "copy") {
        const links = projectSelectionShareLinks(selectionItems);
        if (links.length === 0) return;
        copyTextToClipboard(
          links.join("\n"),
          links.length === 1 ? "Link copied to clipboard" : "Links copied",
        );
        return;
      }
      if (actionId === "create-review") {
        onCreatePullRequest?.();
      }
    },
    [onChatWithAgent, onCreatePullRequest, selection, selectionItems],
  );

  return (
    <div className="w-full" data-testid="projects-selection-actions">
      <h2
        className="flex h-7 min-w-0 items-center truncate text-sm font-normal text-muted-foreground/70"
        data-testid="projects-overview-context-title"
      >
        {presentation.title}
      </h2>
      <div className="space-y-0.5 pt-2">
        {presentation.actions
          .filter(
            (action) =>
              action.id !== "create-review" || Boolean(onCreatePullRequest),
          )
          .map((action) => {
            if (action.id === "discuss") {
              return (
                <ProjectSelectionDiscussAction
                  items={selectionItems}
                  key={action.id}
                  onSelectChannel={discussInChannel}
                />
              );
            }
            const Icon = selectionActionIcon(action.id);
            return (
              <Button
                className="-mx-2 h-7 w-[calc(100%+1rem)] justify-start gap-3 rounded-md px-2 text-left text-sm font-normal hover:bg-muted/70 [&_svg]:h-4 [&_svg]:w-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground"
                data-testid={action.testId}
                key={action.id}
                onClick={() => handleAction(action.id)}
                size="sm"
                type="button"
                variant="ghost"
              >
                <Icon className="h-3.5 w-3.5" />
                {action.label}
              </Button>
            );
          })}
        <Button
          className="-mx-2 h-7 w-[calc(100%+1rem)] justify-start gap-3 rounded-md px-2 text-left text-sm font-normal hover:bg-muted/70 [&_svg]:h-4 [&_svg]:w-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground"
          data-testid="projects-selection-clear"
          onClick={() => selection?.clear()}
          size="sm"
          type="button"
          variant="ghost"
        >
          <X className="h-3.5 w-3.5" />
          Clear selection
        </Button>
      </div>
    </div>
  );
}
