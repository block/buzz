import { toast } from "sonner";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { ManagedAgent } from "@/shared/api/types";

export function showAgentProfileSyncWarning(
  agentName: string,
  profileSyncError: string | null,
) {
  if (!profileSyncError) return;
  toast.warning(
    `${agentName} was saved, but relay profile sync failed: ${profileSyncError}. The relay may still show the old name — restart the agent to retry the sync.`,
  );
}

/**
 * Offer a start after saving an agent that is not running.
 *
 * The auto-restart policy deliberately never fires for a stopped or failing
 * agent (a broken agent must not auto-loop), so an edit made specifically to
 * FIX one silently waits for a manual start. This says so and offers the start,
 * rather than relying on the user to know the policy. A running agent gets
 * nothing — the policy already covers it.
 */
export function showAgentSavedWhileStoppedToast(
  agent: ManagedAgent,
  start: (
    pubkey: string,
    handlers: { onSuccess: () => void; onError: (error: unknown) => void },
  ) => void,
) {
  if (isManagedAgentActive(agent)) return;
  const name = agent.name;
  toast(`${name} saved while stopped.`, {
    action: {
      label: "Start now",
      onClick: () =>
        start(agent.pubkey, {
          onSuccess: () => toast.success(`${name} started.`),
          onError: (error) =>
            toast.error(
              error instanceof Error
                ? `${name} failed to start: ${error.message}`
                : `${name} failed to start.`,
            ),
        }),
    },
  });
}
