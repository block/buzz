import * as React from "react";
import { BookUser } from "lucide-react";

import type { CatalogShareLevel } from "@/features/agents/lib/catalogRelay";
import { encodeTeamSnapshotForSend } from "@/shared/api/tauriTeams";
import type { AgentTeam } from "@/shared/api/types";
import { Switch } from "@/shared/ui/switch";

import { SnapshotShareDialog } from "./PersonaShareDialog";
import { teamCatalogCopy } from "./teamLibraryCopy";

type TeamShareDialogProps = {
  catalogShareLevel: CatalogShareLevel;
  isPending: boolean;
  onCatalogShareLevelChange: (shareLevel: CatalogShareLevel) => void;
  onExport: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  team: AgentTeam;
};

export function TeamShareDialog({
  catalogShareLevel,
  isPending,
  onCatalogShareLevelChange,
  onExport,
  onOpenChange,
  open,
  team,
}: TeamShareDialogProps) {
  const encodeSnapshot = React.useCallback(
    (memoryLevel: "none" | "core" | "everything") =>
      encodeTeamSnapshotForSend(team.id, memoryLevel, "png"),
    [team.id],
  );

  return (
    <SnapshotShareDialog
      beforeExport={
        // Built-in teams have no catalog head to publish — `set_team_shared`
        // rejects them backend-side, so the control is absent rather than
        // present-and-failing.
        team.isBuiltin ? null : (
          <section
            className="relative flex min-h-16 w-full items-center gap-3 rounded-2xl bg-background px-5 py-4 shadow-2xl"
            data-testid="team-share-catalog"
          >
            <BookUser className="h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <h3 className="text-sm font-medium">
                {teamCatalogCopy.shareTitle}
              </h3>
              <p className="text-xs text-secondary-foreground/75">
                {teamCatalogCopy.shareDescription}
              </p>
            </div>
            <Switch
              aria-label="Share to catalog"
              checked={catalogShareLevel !== "not-shared"}
              data-testid="team-share-catalog-access"
              disabled={isPending}
              onCheckedChange={(checked) =>
                onCatalogShareLevelChange(checked ? "none" : "not-shared")
              }
              style={{ cursor: "default" }}
            />
          </section>
        )
      }
      displayName={team.name}
      encodeSnapshot={encodeSnapshot}
      hasMemoryOptions
      isPending={isPending}
      onExport={onExport}
      onOpenChange={onOpenChange}
      open={open}
      snapshotKind="team"
      testIdPrefix="team-share"
    />
  );
}
