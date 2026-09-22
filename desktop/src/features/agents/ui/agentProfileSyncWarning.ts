import { toast } from "sonner";

import { i18n } from "@/i18n";

export function showAgentProfileSyncWarning(
  agentName: string,
  profileSyncError: string | null,
) {
  if (!profileSyncError) return;
  toast.warning(
    i18n.t("agents.profile-sync.warning", {
      agentName,
      error: profileSyncError,
    }),
  );
}
