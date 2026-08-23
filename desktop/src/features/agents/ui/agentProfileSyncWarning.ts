import { toast } from "sonner";
import { PRODUCT_NAME } from "@/shared/brand";

export function showAgentProfileSyncWarning(
  agentName: string,
  profileSyncError: string | null,
) {
  if (!profileSyncError) return;
  toast.warning(
    `${agentName} was saved locally, but relay sync failed: ${profileSyncError}. Remote users may still see the previous name or access policy until ${PRODUCT_NAME} retries the sync.`,
  );
}
