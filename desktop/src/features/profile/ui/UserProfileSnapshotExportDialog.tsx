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
  const exportSnapshotMutation = useExportAgentSnapshotMutation();

  return (
    <AgentSnapshotExportDialog
      isSavePending={exportSnapshotMutation.isPending}
      linkedAgentPubkey={linkedAgentPubkey}
      open
      persona={persona}
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
                toast.success(`Exported ${persona.displayName}.`);
                onOpenChange(false);
              }
            },
            onError: (error) => {
              toast.error(
                error instanceof Error
                  ? error.message
                  : "Failed to export agent snapshot.",
              );
            },
          },
        );
      }}
    />
  );
}
