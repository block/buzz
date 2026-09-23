import { Button } from "@/shared/ui/button";
import { AgentCreationPreview } from "./AgentCreationPreview";

export function EditAgentAvatarColumn({
  isSaving,
  onEditLinkedPersona,
  onOpenChange,
  previewAvatarUrl,
  previewLabel,
  setAvatarUrl,
  setIsAvatarUploadPending,
  useOpenClawWorkspace,
}: {
  isSaving: boolean;
  onEditLinkedPersona?: () => void;
  onOpenChange: (open: boolean) => void;
  previewAvatarUrl: string | null;
  previewLabel: string;
  setAvatarUrl: (url: string) => void;
  setIsAvatarUploadPending: (pending: boolean) => void;
  useOpenClawWorkspace: boolean;
}) {
  return (
    <div className="flex flex-col items-center gap-2">
      <AgentCreationPreview
        avatarUrl={previewAvatarUrl}
        hideEditControl
        label={previewLabel}
        onClearAvatar={() => setAvatarUrl("")}
        onUploadPendingChange={setIsAvatarUploadPending}
        onSelectAvatar={setAvatarUrl}
        showOpenClawWorkspaceBadge={useOpenClawWorkspace}
      />
      {onEditLinkedPersona ? (
        <Button
          className="w-full"
          disabled={isSaving}
          onClick={() => {
            onOpenChange(false);
            onEditLinkedPersona();
          }}
          size="sm"
          type="button"
          variant="outline"
        >
          Edit avatar
        </Button>
      ) : (
        <p className="text-center text-xs text-muted-foreground">
          Avatar is shared identity
        </p>
      )}
    </div>
  );
}
