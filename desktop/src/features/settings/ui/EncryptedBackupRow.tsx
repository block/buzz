import * as React from "react";

import {
  BackupTestFlow,
  initialBackupTestProgress,
} from "@/features/onboarding/ui/BackupTestFlow";
import { EncryptedBackupCreator } from "@/features/onboarding/ui/EncryptedBackupCreator";

/**
 * Collapsible settings row shared by the two backup tools. `relative`
 * anchors the backup-test drop overlay (BackupTestFlow) so a file drag takes
 * over the whole row, mirroring the composer treatment.
 */
function ToolRow({
  title,
  description,
  action,
  open,
  onToggle,
  children,
  testId,
}: {
  title: string;
  description: string;
  action: string;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  testId: string;
}) {
  return (
    <div className="relative px-4 py-3" data-testid={testId}>
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 space-y-1">
          <p className="text-sm font-medium">{title}</p>
          <p className="text-sm text-muted-foreground">{description}</p>
        </div>
        <button
          aria-expanded={open}
          aria-label={open ? `Close ${title.toLowerCase()}` : title}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-muted px-3 py-1.5 text-sm font-medium text-foreground transition-colors hover:bg-muted/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          data-testid={`${testId}-toggle`}
          onClick={onToggle}
          type="button"
        >
          {open ? "Close" : action}
        </button>
      </div>
      {open ? <div className="mt-3">{children}</div> : null}
    </div>
  );
}

/**
 * Sibling settings tools for the password-protected key backup: create a new
 * backup, or test any existing backup file. The raw private key never reaches
 * either flow — the password goes to Rust, which returns only the encrypted
 * NIP-49 blob (create) or the derived public identity (test).
 */
export function EncryptedBackupRow() {
  const [createOpen, setCreateOpen] = React.useState(false);
  const [testOpen, setTestOpen] = React.useState(false);
  const [progress, setProgress] = React.useState(initialBackupTestProgress);
  return (
    <>
      <ToolRow
        action="Create backup"
        description="Download a password-protected copy of your identity key."
        onToggle={() => setCreateOpen((open) => !open)}
        open={createOpen}
        testId="profile-encrypted-backup-row"
        title="Create a key backup"
      >
        <EncryptedBackupCreator guidedTest={false} variant="boxed" />
        <p className="mt-3 text-xs leading-5 text-muted-foreground">
          Keep the file private and save its password somewhere safe — Buzz
          cannot reset it. Creating another backup does not invalidate copies
          you saved before.
        </p>
      </ToolRow>
      <ToolRow
        action="Test backup"
        description="Check a backup file and its password, and see which identity it unlocks."
        onToggle={() => setTestOpen((open) => !open)}
        open={testOpen}
        testId="profile-backup-test-row"
        title="Test a key backup"
      >
        <BackupTestFlow
          onProgressChange={setProgress}
          progress={progress}
          variant="boxed"
        />
        <p className="mt-3 text-xs leading-5 text-muted-foreground">
          Backups use the standard NIP-49 format, so this works for backups from
          compatible Nostr apps too.
        </p>
      </ToolRow>
    </>
  );
}
