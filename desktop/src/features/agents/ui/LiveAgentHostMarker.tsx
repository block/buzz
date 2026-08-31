import { usePresenceRuns } from "@/features/presence/usePresenceRuns";
import { AgentHostMarker } from "./AgentHostMarker";

/** Connected boundary for individual detail surfaces. Lists batch their readers. */
export function LiveAgentHostMarker({
  pubkey,
  ...props
}: {
  pubkey: string;
  otherSetup?: boolean;
  className?: string;
  testId?: string;
}) {
  const presence = usePresenceRuns([pubkey]);
  return (
    <AgentHostMarker
      {...props}
      runs={presence.data?.[pubkey.toLowerCase()]}
      now={presence.now}
    />
  );
}
