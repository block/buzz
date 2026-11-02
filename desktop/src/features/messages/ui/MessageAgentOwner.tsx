import { firstName } from "@/features/channels/ui/DmAgentSubtitle";
import { useProfileQuery } from "@/features/profile/hooks";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";

/**
 * What the header says about the sender besides their name: their job title
 * and, for an agent, who manages it ("Social Media Manager · managed by
 * Alex"). Shows only what is known. An agent with neither is marked "Agent"
 * so a person can still tell who is an AI.
 */
export function MessageAgentOwner({
  isAgent = true,
  jobTitle,
  ownerLabel,
  ownerPubkey,
  selfName,
}: {
  isAgent?: boolean;
  jobTitle?: string | null;
  ownerLabel?: string | null;
  ownerPubkey?: string | null;
  selfName?: string | null;
}) {
  // Your own agents say "managed by <your first name>", as the design does.
  const ownProfile = useProfileQuery(ownerLabel === "you" && !selfName);
  const resolvedSelfName = selfName || ownProfile.data?.displayName;
  const managedBy =
    isAgent && ownerPubkey && ownerLabel
      ? ownerLabel === "you"
        ? (firstName(resolvedSelfName) ?? "you")
        : (firstName(ownerLabel) ?? ownerLabel)
      : null;

  if (!jobTitle && !managedBy) {
    return isAgent ? (
      <span
        aria-hidden="true"
        className="colony-agent-badge c3c-sender-meta min-w-0 truncate"
        data-testid="message-agent-owner"
      >
        Agent
      </span>
    ) : null;
  }

  return (
    <span
      className="c3c-sender-meta inline-flex min-w-0 items-baseline gap-x-1"
      data-testid={isAgent ? "message-agent-owner" : "message-job-title"}
    >
      <span className="sr-only">
        {isAgent ? (managedBy ? "Agent managed by" : "Agent") : "Job title"}
      </span>
      {jobTitle ? (
        // The owner's name gives way first; the title only after it.
        <span className="truncate [flex-shrink:0.1]">{jobTitle}</span>
      ) : null}
      {jobTitle && managedBy ? <span aria-hidden="true">·</span> : null}
      {managedBy && ownerPubkey ? (
        <>
          <span aria-hidden="true" className="shrink-0">
            managed by
          </span>
          <UserProfilePopover
            pubkey={ownerPubkey}
            triggerAriaLabel={ownerLabel ?? managedBy}
            triggerElement="span"
          >
            <span className="min-w-0 truncate rounded hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
              {managedBy}
            </span>
          </UserProfilePopover>
        </>
      ) : null}
    </span>
  );
}
