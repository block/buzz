import * as React from "react";
import { Users } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { ParsedTeamPreview } from "@/shared/api/tauriTeams";
import { createPersona } from "@/shared/api/tauriPersonas";
import { promptPreview } from "@/shared/lib/promptPreview";
import {
  ImportStatusIcon,
  type ImportItemStatus,
} from "@/shared/ui/import-status-icon";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type TeamImportDialogProps = {
  fileName: string;
  open: boolean;
  preview: ParsedTeamPreview | null;
  onOpenChange: (open: boolean) => void;
  onComplete: (
    teamName: string,
    teamDescription: string | null,
    personaIds: string[],
  ) => void;
};

export function TeamImportDialog({
  fileName,
  open,
  preview,
  onOpenChange,
  onComplete,
}: TeamImportDialogProps) {
  const [status, setStatus] = React.useState<
    "idle" | "importing" | "done" | "error"
  >("idle");
  const [importedCount, setImportedCount] = React.useState(0);
  const [itemStatuses, setItemStatuses] = React.useState<
    Map<number, ImportItemStatus>
  >(new Map());
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  const personas = preview?.personas ?? [];

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setStatus("idle");
    setImportedCount(0);
    setItemStatuses(new Map());
    setErrorMessage(null);
  }, [open]);

  async function handleImport() {
    if (!preview || personas.length === 0) {
      return;
    }

    setStatus("importing");
    setErrorMessage(null);

    const initialStatuses = new Map<number, ImportItemStatus>();
    for (let i = 0; i < personas.length; i++) {
      initialStatuses.set(i, "pending");
    }
    setItemStatuses(new Map(initialStatuses));

    const personaIds: string[] = [];
    let completed = 0;

    for (let i = 0; i < personas.length; i++) {
      const persona = personas[i];

      setItemStatuses((prev) => {
        const next = new Map(prev);
        next.set(i, "importing");
        return next;
      });

      try {
        const created = await createPersona({
          displayName: persona.display_name,
          systemPrompt: persona.system_prompt,
          avatarUrl: persona.avatar_url ?? undefined,
        });
        personaIds.push(created.id);
        completed += 1;
        setImportedCount(completed);
        setItemStatuses((prev) => {
          const next = new Map(prev);
          next.set(i, "done");
          return next;
        });
      } catch (error) {
        setItemStatuses((prev) => {
          const next = new Map(prev);
          next.set(i, "error");
          return next;
        });
        setStatus("error");
        setErrorMessage(
          `Imported ${completed} of ${personas.length} personas. Failed on '${persona.display_name}': ${error instanceof Error ? error.message : String(error)}. Already-imported personas are saved.`,
        );
        return;
      }
    }

    setStatus("done");
    onComplete(preview.name, preview.description, personaIds);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="flex max-h-[80vh] max-w-2xl flex-col overflow-hidden p-0">
        <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
          <DialogTitle>Import Team</DialogTitle>
          <DialogDescription>
            Preview the team from {fileName || "file"} before importing.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          {preview ? (
            <div className="space-y-4">
              <div className="flex items-center gap-3 rounded-lg border border-border/60 bg-card/80 px-4 py-3">
                <Users className="h-4 w-4 shrink-0 text-muted-foreground" />
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-semibold tracking-tight">
                    {preview.name}
                  </p>
                  {preview.description ? (
                    <p className="text-xs text-muted-foreground">
                      {preview.description}
                    </p>
                  ) : null}
                </div>
                <span className="text-xs text-muted-foreground">
                  {personas.length}{" "}
                  {personas.length === 1 ? "persona" : "personas"}
                </span>
              </div>

              <div className="space-y-1">
                <p className="text-sm font-medium">Personas to import</p>
                <p className="text-xs text-muted-foreground">
                  Each persona will be created, then grouped into a new team.
                </p>
              </div>

              <div className="space-y-1">
                {personas.map((persona, index) => (
                  <div
                    className="flex items-center gap-3 rounded-lg border border-border/60 bg-card/80 px-3 py-2.5"
                    // biome-ignore lint/suspicious/noArrayIndexKey: static list from imported JSON file, never reordered
                    key={index}
                  >
                    <ProfileAvatar
                      avatarUrl={persona.avatar_url}
                      className="h-8 w-8 rounded-lg text-xs"
                      label={persona.display_name}
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-semibold tracking-tight">
                        {persona.display_name}
                      </p>
                      <p className="truncate text-xs text-muted-foreground">
                        {promptPreview(persona.system_prompt)}
                      </p>
                    </div>
                    <ImportStatusIcon status={itemStatuses.get(index)} />
                  </div>
                ))}
              </div>
            </div>
          ) : null}

          {errorMessage ? (
            <p className="mt-4 rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
              {errorMessage}
            </p>
          ) : null}
        </div>

        <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
          <Button
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            disabled={
              !preview ||
              personas.length === 0 ||
              status === "importing" ||
              status === "done" ||
              status === "error"
            }
            onClick={() => void handleImport()}
            size="sm"
            type="button"
          >
            {status === "importing"
              ? `Importing ${importedCount}/${personas.length}...`
              : `Import team (${personas.length} personas)`}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
