import { Button } from "@/shared/ui/button";

import type { RapidSaveMode } from "./agentRapidTest";

export function AgentRapidActionsFooter({
  canSubmit,
  canRestart,
  canSmoke,
  hasRapidTestSelection,
  isAvatarUploadPending,
  isSubmitPending,
  rapidAction,
  onCancel,
  onSubmit,
}: {
  canSubmit: boolean;
  canRestart: boolean;
  canSmoke: boolean;
  hasRapidTestSelection: boolean;
  isAvatarUploadPending: boolean;
  isSubmitPending: boolean;
  rapidAction: RapidSaveMode | null;
  onCancel: () => void;
  onSubmit: (mode: RapidSaveMode) => void;
}) {
  return (
    <div
      aria-busy={isSubmitPending}
      className="flex w-full flex-wrap items-center justify-end gap-2"
    >
      <Button
        disabled={isAvatarUploadPending}
        onClick={onCancel}
        type="button"
        variant="outline"
      >
        Cancel
      </Button>
      <Button
        data-testid="edit-agent-dialog-submit"
        disabled={!canSubmit}
        onClick={() => onSubmit("save")}
        type="button"
        variant="outline"
      >
        {rapidAction === "save" ? "Saving..." : "Save changes"}
      </Button>
      <Button
        data-testid="edit-agent-dialog-restart"
        disabled={!canSubmit || !canRestart}
        onClick={() => onSubmit("restart")}
        type="button"
        variant="outline"
      >
        {rapidAction === "restart" ? "Restarting..." : "Save & restart"}
      </Button>
      <Button
        data-testid="edit-agent-dialog-smoke"
        disabled={!canSubmit || !canSmoke || !hasRapidTestSelection}
        onClick={() => onSubmit("smoke")}
        type="button"
      >
        {rapidAction === "smoke" ? "Posting test..." : "Save, restart & test"}
      </Button>
    </div>
  );
}
