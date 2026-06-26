import * as React from "react";
import { Archive, ArchiveRestore } from "lucide-react";

import type { IdentityArchiveActions } from "@/features/identity-archive/hooks";
import { ArchiveConfirmDialog } from "@/features/profile/ui/ArchiveConfirmDialog";
import { Button } from "@/shared/ui/button";

export function ProfileManageArchiveSection({
  archiveActions,
  isBot,
  onGoToAgents,
}: {
  archiveActions: IdentityArchiveActions;
  isBot: boolean;
  onGoToAgents: () => void;
}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);

  const archiveLabel = isBot ? "Archive agent" : "Archive identity";
  const unarchiveLabel = isBot ? "Unarchive agent" : "Unarchive identity";

  return (
    <section className="flex flex-col gap-2">
      <h4 className="text-xs font-medium uppercase tracking-wider text-muted-foreground/70">
        Manage
      </h4>
      {archiveActions.isArchived ? (
        <Button
          className="w-full"
          data-testid="user-profile-unarchive-identity"
          disabled={archiveActions.isPending}
          onClick={archiveActions.unarchive}
          type="button"
          variant="secondary"
        >
          <ArchiveRestore className="h-4 w-4" />
          {archiveActions.isPending ? "Unarchiving…" : unarchiveLabel}
        </Button>
      ) : (
        <Button
          className="w-full"
          data-testid="user-profile-archive-identity"
          disabled={archiveActions.isPending}
          onClick={() => setConfirmOpen(true)}
          type="button"
          variant="secondary"
        >
          <Archive className="h-4 w-4" />
          {archiveActions.isPending ? "Archiving…" : archiveLabel}
        </Button>
      )}
      <ArchiveConfirmDialog
        isBot={isBot}
        isPending={archiveActions.isPending}
        onConfirm={() => {
          archiveActions.archive();
          setConfirmOpen(false);
        }}
        onGoToAgents={onGoToAgents}
        onOpenChange={setConfirmOpen}
        open={confirmOpen}
      />
    </section>
  );
}
