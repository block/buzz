// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { toast } from "sonner";

export function showAgentProfileSyncWarning(
  agentName: string,
  profileSyncError: string | null,
) {
  if (!profileSyncError) return;
  toast.warning(
    uiT(
      "{0} was saved locally, but relay sync failed: {1}. Remote users may still see the previous name or access policy until Buzz retries the sync.",
      { p0: agentName, p1: profileSyncError },
    ),
  );
}
