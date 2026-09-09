import { Bot } from "lucide-react";

import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";

export function MessageAgentOwner({
  ownerLabel,
  ownerPubkey,
}: {
  ownerLabel?: string | null;
  ownerPubkey?: string | null;
}) {
  return (
    <span
      className="inline-flex min-w-0 max-w-56 items-baseline gap-1 text-message-timestamp text-muted-foreground/65"
      data-testid="message-agent-owner"
    >
      <span className="sr-only">
        {ownerLabel ? "Agent managed by" : "Agent; owner unavailable"}
      </span>
      <Bot aria-hidden="true" className="h-3.5 w-3.5 shrink-0 self-center" />
      {ownerPubkey && ownerLabel ? (
        <>
          <span aria-hidden="true" className="shrink-0">
            managed by
          </span>
          <UserProfilePopover
            pubkey={ownerPubkey}
            triggerAriaLabel={ownerLabel}
            triggerElement="span"
          >
            <span className="min-w-0 truncate rounded font-semibold text-foreground/85 hover:text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
              {ownerLabel}
            </span>
          </UserProfilePopover>
        </>
      ) : (
        <span aria-hidden="true" className="min-w-0 truncate">
          owner unavailable
        </span>
      )}
    </span>
  );
}
