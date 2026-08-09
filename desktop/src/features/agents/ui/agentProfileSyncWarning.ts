import { toast } from "sonner";

export function showAgentProfileSyncWarning(
  agentName: string,
  profileSyncError: string | null,
) {
  if (!profileSyncError) return;
  toast.warning(
    `${agentName} was saved, but relay profile sync failed: ${profileSyncError}. The relay may still show the old name — restart the agent to retry the sync.`,
  );
}
