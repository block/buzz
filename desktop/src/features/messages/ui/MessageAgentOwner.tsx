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
      className="inline-flex min-w-0 max-w-56 items-baseline gap-1 text-2xs font-normal leading-4 text-muted-foreground"
      data-testid="message-agent-owner"
    >
      <span className="sr-only">
        {ownerLabel ? "Agent managed by" : "Agent; owner unavailable"}
      </span>
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
            <span className="min-w-0 truncate rounded font-medium text-muted-foreground hover:text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
              {ownerLabel}
            </span>
          </UserProfilePopover>
        </>
      ) : (
        <span aria-hidden="true" className="truncate">
          owner unavailable
        </span>
      )}
    </span>
  );
}
