import { Copy, EllipsisVertical, Trash2 } from "lucide-react";

import {
  registeredAgentRoleSummary,
  resolveRegisteredAgentDisplay,
} from "@/features/agents/lib/registeredAgentCards";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type { RegisteredAgentReference } from "@/shared/api/tauriRegisteredAgents";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { AgentIdentityCard } from "./AgentIdentityCard";

export function RegisteredAgentIdentityCard({
  isPending,
  onOpenProfile,
  onRemove,
  reference,
}: {
  isPending: boolean;
  onOpenProfile: (pubkey: string) => void;
  onRemove: (reference: RegisteredAgentReference) => void;
  reference: RegisteredAgentReference;
}) {
  const profileQuery = useUserProfileQuery(reference.pubkey);
  const display = resolveRegisteredAgentDisplay({
    reference,
    profile: profileQuery.data,
  });
  const summary = registeredAgentRoleSummary(reference.roleSummary);

  async function copyPubkey() {
    try {
      await navigator.clipboard?.writeText(reference.pubkey);
    } catch {
      // Clipboard availability is best-effort; never leak a rejected browser
      // permission promise from a menu event.
    }
  }

  return (
    <AgentIdentityCard
      actions={
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={`Actions for ${display.label}`}
              data-testid={`registered-agent-actions-${reference.pubkey}`}
              disabled={isPending}
              size="icon"
              type="button"
              variant="ghost"
            >
              <EllipsisVertical className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onSelect={(event) => {
                event.preventDefault();
                void copyPubkey();
              }}
            >
              <Copy />
              Copy full pubkey
            </DropdownMenuItem>
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              onSelect={() => onRemove(reference)}
            >
              <Trash2 />
              Remove reference
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      }
      ariaLabel={`${display.label} externally managed agent profile, public key ${reference.pubkey}`}
      avatarUrl={display.avatarUrl}
      dataTestId={`registered-agent-${reference.pubkey}`}
      label={display.label}
      modelLabel={summary}
      onClick={() => onOpenProfile(reference.pubkey)}
      statusBadge={
        <span className="block break-all rounded-md bg-background/80 px-2 py-1 text-3xs font-mono text-muted-foreground">
          {truncatePubkey(reference.pubkey)}
        </span>
      }
    />
  );
}
