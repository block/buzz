import * as React from "react";
import { FolderOpen, X } from "lucide-react";
import { toast } from "sonner";

import { pickAgentWorkingDirectory } from "@/shared/api/tauriManagedAgents";
import { Button } from "@/shared/ui/button";

/** Local-only process-CWD picker shared by agent create and instance edit. */
export function AgentWorkingFolderField({
  disabled,
  onChange,
  value,
}: {
  disabled?: boolean;
  onChange: (value: string) => void;
  value: string;
}) {
  const [isPicking, setIsPicking] = React.useState(false);
  const selected = value.trim();

  async function chooseFolder() {
    setIsPicking(true);
    try {
      const path = await pickAgentWorkingDirectory();
      if (path) onChange(path);
    } catch (error) {
      toast.error("Couldn’t choose working folder", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsPicking(false);
    }
  }

  return (
    <div className="space-y-1.5" data-testid="agent-working-folder-field">
      <p className="text-sm font-medium text-foreground">Working folder</p>
      <div className="flex items-center gap-2">
        <Button
          aria-label={
            selected
              ? "Choose a different working folder"
              : "Choose working folder"
          }
          className="min-w-0 flex-1 justify-start font-normal"
          data-testid="agent-working-folder-path"
          disabled={disabled || isPicking}
          onClick={() => void chooseFolder()}
          title={selected || undefined}
          type="button"
          variant="outline"
        >
          <FolderOpen aria-hidden className="h-4 w-4 shrink-0" />
          <span className="truncate">
            {isPicking ? "Choosing…" : selected || "Buzz workspace (default)"}
          </span>
        </Button>
        {selected ? (
          <Button
            aria-label="Clear working folder"
            disabled={disabled || isPicking}
            onClick={() => onChange("")}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X aria-hidden className="h-4 w-4" />
          </Button>
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground">
        Local files and relative paths start here. This does not limit access to
        other files on your computer.
      </p>
    </div>
  );
}
