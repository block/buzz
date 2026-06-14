import { Archive, ArchiveRestore } from "lucide-react";
import { useState } from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIdentityArchive } from "@/features/identity-archive/hooks";
import { Button } from "@/shared/ui/button";
import { ArchiveConfirmDialog } from "./ArchiveConfirmDialog";

// NIP-IA archive / unarchive lives in its own section under the quick-actions
// row. The relay verifies authority (self / admin / OA-owner) on submit; the
// `canArchive` gate upstream is a UX guard so the button only renders when at
// least one path will be accepted.
export function ProfileManageSection({
  isBot,
  pubkey,
}: {
  isBot: boolean;
  pubkey: string;
}) {
  const { isArchived, isPending, archive, unarchive } =
    useIdentityArchive(pubkey);
  const { goAgents } = useAppNavigation();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const archiveLabel = isBot ? "Archive agent" : "Archive identity";
  const unarchiveLabel = isBot ? "Unarchive agent" : "Unarchive identity";

  const handleConfirm = () => {
    archive();
    setConfirmOpen(false);
  };

  return (
    <section className="flex flex-col gap-2">
      <h4 className="text-xs font-medium uppercase tracking-wider text-muted-foreground/70">
        Manage
      </h4>
      {isArchived ? (
        <Button
          className="w-full"
          data-testid="user-profile-unarchive-identity"
          disabled={isPending}
          onClick={unarchive}
          type="button"
          variant="secondary"
        >
          <ArchiveRestore className="h-4 w-4" />
          {isPending ? "Unarchiving…" : unarchiveLabel}
        </Button>
      ) : (
        <Button
          className="w-full"
          data-testid="user-profile-archive-identity"
          disabled={isPending}
          onClick={() => setConfirmOpen(true)}
          type="button"
          variant="secondary"
        >
          <Archive className="h-4 w-4" />
          {isPending ? "Archiving…" : archiveLabel}
        </Button>
      )}
      <ArchiveConfirmDialog
        isBot={isBot}
        isPending={isPending}
        onConfirm={handleConfirm}
        onGoToAgents={() => {
          void goAgents();
        }}
        onOpenChange={setConfirmOpen}
        open={confirmOpen}
      />
    </section>
  );
}
