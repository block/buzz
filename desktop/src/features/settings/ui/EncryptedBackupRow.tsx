import * as React from "react";

import { EncryptedBackupCreator } from "@/features/onboarding/ui/EncryptedBackupCreator";

/**
 * Collapsible row for creating and testing a password-protected key backup.
 * The raw private key never reaches this flow — the password goes to Rust,
 * which returns the persisted `ncryptsec1…` blob.
 */
export function EncryptedBackupRow() {
  const [isOpen, setIsOpen] = React.useState(false);

  return (
    // `relative`: anchors the backup-test drop overlay (BackupTestFlow) so a
    // file drag takes over this whole row, mirroring the composer treatment.
    <div
      className="relative px-4 py-3"
      data-testid="profile-encrypted-backup-row"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 space-y-1">
          <p className="text-sm font-medium">Password-protected key backup</p>
          <p className="text-sm text-muted-foreground">
            Download an encrypted copy of your identity key, then test the file
            and password before relying on it.
          </p>
        </div>
        <button
          aria-expanded={isOpen}
          aria-label={
            isOpen
              ? "Close password-protected key backup"
              : "Create password-protected key backup"
          }
          className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-muted px-3 py-1.5 text-sm font-medium text-foreground transition-colors hover:bg-muted/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          data-testid="profile-encrypted-backup-toggle"
          onClick={() => setIsOpen((open) => !open)}
          type="button"
        >
          {isOpen ? "Close" : "Create backup"}
        </button>
      </div>
      {isOpen ? (
        <div className="mt-3">
          <EncryptedBackupCreator variant="boxed" />
          <p className="mt-3 text-xs leading-5 text-muted-foreground">
            Keep the downloaded file private and save its password somewhere
            safe. Buzz cannot reset the password. Creating another backup does
            not invalidate copies you saved before.
          </p>
        </div>
      ) : null}
    </div>
  );
}
