// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { useExportAgentSnapshotMutation } from "@/features/agents/hooks";
import { AgentSnapshotExportDialog } from "@/features/agents/ui/AgentSnapshotExportDialog";
import type { AgentPersona } from "@/shared/api/types";
import { toast } from "sonner";

export function UserProfileSnapshotExportDialog({
  persona,
  linkedAgentPubkey,
  onOpenChange,
}: {
  persona: AgentPersona;
  linkedAgentPubkey: string | null;
  onOpenChange: (open: boolean) => void;
}) {
  useUiLanguage();

  const exportSnapshotMutation = useExportAgentSnapshotMutation();

  return (
    <AgentSnapshotExportDialog
      agentName={persona.displayName}
      isSavePending={exportSnapshotMutation.isPending}
      linkedAgentPubkey={linkedAgentPubkey}
      open
      onOpenChange={onOpenChange}
      onSaveFile={(memoryLevel, format) => {
        exportSnapshotMutation.mutate(
          {
            id: persona.id,
            memoryLevel,
            format,
            memorySourcePubkey: linkedAgentPubkey,
            avatarUrl: persona.avatarUrl,
          },
          {
            onSuccess: (saved) => {
              if (saved) {
                toast.success(
                  uiT("Exported {0}.", { p0: persona.displayName }),
                );
                onOpenChange(false);
              }
            },
            onError: (error) => {
              toast.error(
                error instanceof Error
                  ? error.message
                  : uiT("Failed to export agent snapshot."),
              );
            },
          },
        );
      }}
    />
  );
}
