import { Download, Eye, EyeOff, ShieldCheck } from "lucide-react";
import * as React from "react";

import { NsecMaskedDisplay } from "@/features/onboarding/ui/NsecMaskedDisplay";
import {
  BACKUP_AVAILABILITY_MS,
  useEncryptedBackup,
} from "@/features/settings/EncryptedBackupProvider";
import {
  BackupTestFlow,
  initialBackupTestProgress,
} from "@/features/settings/ui/BackupTestFlow";
import { EncryptedBackupCreator } from "@/features/settings/ui/EncryptedBackupCreator";
import { getNsec } from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

function BackupAvailabilityFill({
  availableUntil,
}: {
  availableUntil: number;
}) {
  const [{ durationMs, initialWidth }] = React.useState(() => {
    const remainingMs = Math.max(0, availableUntil - Date.now());
    return {
      durationMs: remainingMs,
      initialWidth: Math.min(100, (remainingMs / BACKUP_AVAILABILITY_MS) * 100),
    };
  });
  const [width, setWidth] = React.useState(initialWidth);

  React.useEffect(() => {
    const frame = window.requestAnimationFrame(() => setWidth(0));
    return () => window.cancelAnimationFrame(frame);
  }, []);

  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute inset-y-0 left-0 bg-primary-foreground/15 transition-[width] ease-linear"
      data-testid="encrypted-backup-availability-fill"
      style={{
        transitionDuration: `${durationMs}ms`,
        width: `${width}%`,
      }}
    />
  );
}

/**
 * Collapsible private-key row with backup actions. The nsec is fetched only
 * while expanded; encrypted backup state lives in the app-level provider.
 */
export function PrivateKeyBackupRow() {
  const [isOpen, setIsOpen] = React.useState(false);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [createOpen, setCreateOpen] = React.useState(false);
  const [testOpen, setTestOpen] = React.useState(false);
  const [testProgress, setTestProgress] = React.useState(
    initialBackupTestProgress,
  );
  const {
    availableUntil,
    backupAvailable,
    downloadBackup,
    isSaving,
    startNewBackup,
  } = useEncryptedBackup();
  const fetchCancelledRef = React.useRef(false);

  React.useEffect(() => {
    return () => {
      fetchCancelledRef.current = true;
      setNsec(null);
    };
  }, []);

  async function handleReveal() {
    if (!isOpen) {
      fetchCancelledRef.current = false;
      setIsOpen(true);
      setIsLoading(true);
      setLoadError(null);
      try {
        const value = await getNsec();
        if (!fetchCancelledRef.current) setNsec(value);
      } catch (err) {
        if (!fetchCancelledRef.current)
          setLoadError(
            err instanceof Error
              ? err.message
              : "Failed to retrieve private key.",
          );
      } finally {
        if (!fetchCancelledRef.current) setIsLoading(false);
      }
      return;
    }

    fetchCancelledRef.current = true;
    setNsec(null);
    setIsOpen(false);
  }

  function handleCreateBackup() {
    if (backupAvailable) startNewBackup();
    setCreateOpen(true);
  }

  function handleTestOpenChange(open: boolean) {
    setTestOpen(open);
    if (!open) setTestProgress(initialBackupTestProgress);
  }

  return (
    <>
      <div className="px-4 py-3" data-testid="profile-private-key-row">
        <div className="flex items-center justify-between gap-4">
          <p className="text-sm font-medium">Private key</p>
          <div className="flex shrink-0 items-center gap-2">
            {backupAvailable ? (
              <Button
                className="relative overflow-hidden rounded-full"
                data-testid="encrypted-backup-download"
                disabled={isSaving}
                onClick={() => void downloadBackup()}
                type="button"
              >
                {availableUntil !== null ? (
                  <BackupAvailabilityFill availableUntil={availableUntil} />
                ) : null}
                <span className="relative z-10">Download backup</span>
              </Button>
            ) : null}
            <Button
              aria-label={isOpen ? "Hide private key" : "Reveal private key"}
              className="rounded-full"
              data-testid="profile-private-key-toggle"
              onClick={() => void handleReveal()}
              type="button"
              variant="secondary"
            >
              {isOpen ? (
                <>
                  <EyeOff className="h-4 w-4 shrink-0" />
                  Hide
                </>
              ) : (
                <>
                  <Eye className="h-4 w-4 shrink-0" />
                  Reveal
                </>
              )}
            </Button>
          </div>
        </div>
        {isOpen ? (
          <div className="mt-2">
            {isLoading ? (
              <p className="text-sm text-muted-foreground">Loading…</p>
            ) : loadError ? (
              <p className="text-sm text-destructive">{loadError}</p>
            ) : nsec ? (
              <NsecMaskedDisplay
                actions={[
                  {
                    icon: <Download aria-hidden="true" />,
                    label: "Create backup",
                    onSelect: handleCreateBackup,
                    testId: "private-key-create-backup",
                  },
                  {
                    icon: <ShieldCheck aria-hidden="true" />,
                    label: "Test backup",
                    onSelect: () => setTestOpen(true),
                    testId: "private-key-test-backup",
                  },
                ]}
                nsec={nsec}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      <EncryptedBackupCreator onOpenChange={setCreateOpen} open={createOpen} />
      <Dialog onOpenChange={handleTestOpenChange} open={testOpen}>
        <DialogContent className="max-w-lg" data-testid="backup-test-dialog">
          <DialogHeader className="pr-8">
            <DialogTitle>Test a key backup</DialogTitle>
            <DialogDescription>
              Confirm that a backup file and its password can unlock an
              identity.
            </DialogDescription>
          </DialogHeader>
          <BackupTestFlow
            onProgressChange={setTestProgress}
            progress={testProgress}
          />
          <p className="text-xs leading-5 text-muted-foreground">
            Backups use the standard NIP-49 format, so this works for backups
            from compatible Nostr apps too.
          </p>
        </DialogContent>
      </Dialog>
    </>
  );
}
