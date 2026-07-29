import { AtSign, ChevronDown, MessagesSquare } from "lucide-react";

import {
  agentResponseEmoji,
  agentResponseLabel,
} from "@/features/channels/lib/agentResponsePolicy";
import type { AgentResponsePolicy } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { cn } from "@/shared/lib/cn";

export function ChannelAgentResponseSettings({
  disabled,
  onPolicyChange,
  policy,
  testIdPrefix,
}: {
  disabled?: boolean;
  onPolicyChange: (policy: AgentResponsePolicy) => void;
  policy: AgentResponsePolicy;
  testIdPrefix: string;
}) {
  const policyLabel = `${agentResponseEmoji(policy)} ${agentResponseLabel(policy)}`;

  return (
    <div
      className={cn(
        "flex min-h-12 items-center justify-between gap-4 rounded-xl border border-input bg-background px-3 py-3",
        disabled && "opacity-50",
      )}
      data-testid={`${testIdPrefix}-agent-response-container`}
    >
      <div className="min-w-0">
        <div className="text-sm font-medium text-foreground">Agent replies</div>
        <div className="text-xs text-muted-foreground">
          Applies to every agent in this channel
        </div>
      </div>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={`Agent replies: ${policyLabel}`}
            className="-mr-2.5 ml-auto h-9 w-fit justify-end px-2.5 text-right text-sm font-medium text-foreground hover:bg-muted/50"
            data-testid={`${testIdPrefix}-agent-response`}
            disabled={disabled}
            type="button"
            variant="ghost"
          >
            <span className="text-right">{policyLabel}</span>
            <ChevronDown className="size-4 shrink-0 text-muted-foreground/70" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          onCloseAutoFocus={(event) => event.preventDefault()}
          style={{ minWidth: "var(--radix-dropdown-menu-trigger-width)" }}
        >
          <DropdownMenuRadioGroup
            onValueChange={(value) =>
              onPolicyChange(value === "all" ? "all" : "mentions")
            }
            value={policy}
          >
            <DropdownMenuRadioItem
              data-testid={`${testIdPrefix}-agent-response-option-mentions`}
              value="mentions"
            >
              <AtSign className="mr-2 size-4" />
              🏷️ Only @mentions
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem
              data-testid={`${testIdPrefix}-agent-response-option-all`}
              value="all"
            >
              <MessagesSquare className="mr-2 size-4" />💬 Every message
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
