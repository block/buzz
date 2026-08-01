import { MessageSquarePlus } from "lucide-react";

import { Button } from "@/shared/ui/button";

type AssistantWorkspaceHeaderProps = {
  onNewConversation: () => void;
};

/**
 * Friendly entry point for standard mode. The action deliberately routes into
 * Buzz's existing new-message flow, which can resolve both people and agents
 * from the relay. Keeping that flow shared preserves upstream behavior while
 * presenting the product as an agent workspace instead of a chat server.
 */
export function AssistantWorkspaceHeader({
  onNewConversation,
}: AssistantWorkspaceHeaderProps) {
  return (
    <div className="pb-1" data-testid="assistant-workspace-header">
      <Button
        className="h-auto w-full justify-start gap-3 rounded-xl border border-sidebar-border/60 bg-background/80 px-3 py-2.5 text-left text-sidebar-foreground shadow-xs hover:bg-background"
        data-testid="assistant-new-conversation"
        onClick={onNewConversation}
        type="button"
        variant="outline"
      >
        <MessageSquarePlus className="h-4 w-4 shrink-0" />
        <span className="min-w-0">
          <span className="block truncate text-sm font-medium">
            New conversation
          </span>
          <span className="block truncate text-2xs font-normal text-muted-foreground">
            Talk with a person or agent
          </span>
        </span>
      </Button>
    </div>
  );
}
