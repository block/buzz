import { useExportAgentSnapshotMutation } from "@/features/agents/hooks";
import { AgentSnapshotExportDialog } from "@/features/agents/ui/AgentSnapshotExportDialog";
import type { AgentPersona } from "@/shared/api/types";
import { toast } from "sonner";

export function UserProfileSnapshotExportDialog({
  agentAvatarUrl,
  persona,
  linkedAgentPubkey,
  onOpenChange,
}: {
  agentAvatarUrl: string | null;
  persona: AgentPersona;
  linkedAgentPubkey: string | null;
  onOpenChange: (open: boolean) => void;
}) {
  const exportSnapshotMutation = useExportAgentSnapshotMutation();

  return (
    <AgentSnapshotExportDialog
      agentId={linkedAgentPubkey ?? persona.id}
      agentAvatarUrl={agentAvatarUrl}
      agentName={persona.displayName}
      isSavePending={exportSnapshotMutation.isPending}
      linkedAgentPubkey={linkedAgentPubkey}
      open
      onOpenChange={onOpenChange}
      onSaveFile={(memoryLevel, format, cardPngDataUrl) => {
        exportSnapshotMutation.mutate(
          {
            id: persona.id,
            memoryLevel,
            format,
            memorySourcePubkey: linkedAgentPubkey,
            avatarUrl: agentAvatarUrl,
            cardPngDataUrl,
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
