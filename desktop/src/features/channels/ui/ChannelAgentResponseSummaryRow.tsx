import {
  AtSign,
  ChevronDown,
  LoaderCircle,
  MessagesSquare,
} from "lucide-react";

import {
  agentResponseEmoji,
  agentResponseLabel,
} from "@/features/channels/lib/agentResponsePolicy";
import type { AgentResponsePolicy } from "@/shared/api/types";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function ChannelAgentResponseSummaryRow({
  canManage,
  isPending,
  onPolicyChange,
  policy,
}: {
  canManage: boolean;
  isPending: boolean;
  onPolicyChange: (policy: AgentResponsePolicy) => void;
  policy: AgentResponsePolicy;
}) {
  const status = `${agentResponseEmoji(policy)} ${agentResponseLabel(policy)}`;
  const content = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <MessagesSquare className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-xs font-medium text-foreground">
          Agent replies
        </span>
        <span className="mt-0.5 block truncate text-sm text-muted-foreground">
          {status}
        </span>
      </span>
    </>
  );

  if (!canManage) {
    return (
      <div
        className="flex w-full items-center gap-3 px-4 py-3"
        data-testid="channel-management-agent-response-summary"
      >
        {content}
      </div>
    );
  }

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Agent replies: ${status}`}
          className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 disabled:cursor-wait disabled:opacity-60"
          data-testid="channel-management-agent-response-summary"
          disabled={isPending}
          type="button"
        >
          {content}
          {isPending ? (
            <LoaderCircle className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
          ) : (
            <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
          )}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuRadioGroup
          onValueChange={(value) =>
            onPolicyChange(value === "all" ? "all" : "mentions")
          }
          value={policy}
        >
          <DropdownMenuRadioItem
            data-testid="channel-management-agent-response-summary-option-mentions"
            value="mentions"
          >
            <AtSign className="mr-2 size-4" />
            🏷️ Only @mentions
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem
            data-testid="channel-management-agent-response-summary-option-all"
            value="all"
          >
            <MessagesSquare className="mr-2 size-4" />💬 Every message
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
