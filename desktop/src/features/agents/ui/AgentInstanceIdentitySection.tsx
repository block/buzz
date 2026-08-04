import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { AgentCreationPreview } from "./AgentCreationPreview";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

export function AgentInstanceIdentitySection({
  avatarUrl,
  disabled,
  name,
  onEditTemplate,
  onNameChange,
  onSelectAvatar,
  onUploadPendingChange,
}: {
  avatarUrl: string | null;
  disabled: boolean;
  name: string;
  onEditTemplate?: () => void;
  onNameChange: (name: string) => void;
  onSelectAvatar: (avatarUrl: string) => void;
  onUploadPendingChange: (pending: boolean) => void;
}) {
  const previewLabel = name.trim() || "Agent name";

  return (
    <>
      <div className="flex flex-col items-center gap-2">
        <AgentCreationPreview
          avatarUrl={avatarUrl}
          hideEditControl
          label={previewLabel}
          onUploadPendingChange={onUploadPendingChange}
          onSelectAvatar={onSelectAvatar}
          variant="compact"
        />
        {onEditTemplate ? (
          <Button
            className="w-full"
            onClick={onEditTemplate}
            size="sm"
            type="button"
            variant="outline"
          >
            Edit template
          </Button>
        ) : (
          <p className="text-center text-xs text-muted-foreground">
            Avatar is shared identity
          </p>
        )}
      </div>

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-name"
        >
          Agent name
        </label>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="edit-agent-name"
            onChange={(event) => onNameChange(event.target.value)}
            placeholder="Agent name"
            value={name}
          />
        </div>
      </div>
    </>
  );
}
