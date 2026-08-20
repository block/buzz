import * as React from "react";

import { encodeTeamSnapshotForSend } from "@/shared/api/tauriTeams";
import type { AgentTeam } from "@/shared/api/types";

import { SnapshotShareDialog } from "./PersonaShareDialog";

type TeamShareDialogProps = {
  isPending: boolean;
  onExport: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  team: AgentTeam;
};

export function TeamShareDialog({
  isPending,
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
