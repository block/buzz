import { Cloud } from "lucide-react";

import { useIsOtherSetupAgent } from "../useKnownAgentPubkeys";

import { cn } from "@/shared/lib/cn";

const OTHER_SETUP_LABEL = "Not managed on this device";

export function OtherSetupAgentMarker({
  className,
  testId,
}: {
  className?: string;
  testId?: string;
}) {
  return (
    <span
      aria-label={OTHER_SETUP_LABEL}
      className={cn("inline-flex shrink-0", className)}
      data-testid={testId}
      role="img"
      title={OTHER_SETUP_LABEL}
    >
      <Cloud aria-hidden="true" className="h-3 w-3" />
    </span>
  );
}

/** Connected marker for identity details; shares the app's directory subscriptions. */
export function AgentManagementMarker({
  pubkey,
  ownerPubkey,
  className,
  testId,
}: {
  pubkey?: string | null;
  ownerPubkey?: string | null;
  className?: string;
  testId?: string;
}) {
  const show = useIsOtherSetupAgent(pubkey, ownerPubkey);
  return show ? (
    <OtherSetupAgentMarker className={className} testId={testId} />
  ) : null;
}
