import { Bot } from "lucide-react";
import * as React from "react";

import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import { Button } from "@/shared/ui/button";
import { PROJECT_CONTEXT_ACTION_BUTTON_CLASS } from "./projectContextActionStyles";
import { ProjectSelectionDiscussAction } from "./ProjectSelectionDiscussAction";
import { useProjectDiscussInChannel } from "./useProjectDiscussInChannel";

export function ProjectWorkItemCommunicationActions({
  item,
  onChatWithAgent,
}: {
  item: ProjectSelectionItem;
  onChatWithAgent: (items: ProjectSelectionItem[]) => void;
}) {
  const items = React.useMemo(() => [item], [item]);
  const discussInChannel = useProjectDiscussInChannel(items);

  return (
    <section
      className="space-y-0.5 pt-2"
      data-testid="project-context-communication-actions"
    >
      <h3 className="text-xs font-normal text-muted-foreground/70">
        Discussion
      </h3>
      <Button
        className={PROJECT_CONTEXT_ACTION_BUTTON_CLASS}
        data-testid="project-context-chat-agent"
        onClick={() => onChatWithAgent(items)}
        size="sm"
        type="button"
        variant="ghost"
      >
        <Bot className="h-3.5 w-3.5" />
        Chat with an agent
      </Button>
      <ProjectSelectionDiscussAction
        items={items}
        onSelectChannel={discussInChannel}
        testIdPrefix="project-context"
      />
    </section>
  );
}
