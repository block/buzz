import * as React from "react";
import { EncryptedBackupCreator } from "@/features/onboarding/ui/EncryptedBackupCreator";
import {
  BackupTestFlow,
  initialBackupTestProgress,
} from "@/features/onboarding/ui/BackupTestFlow";

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
          data-testid={`${testId}-toggle`}
          className="inline-flex shrink-0 rounded-full bg-muted px-3 py-1.5 text-sm font-medium hover:bg-muted/80 focus-visible:ring-2 focus-visible:ring-ring"
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

export function EncryptedBackupRow() {
  const [createOpen, setCreateOpen] = React.useState(false);
  const [testOpen, setTestOpen] = React.useState(false);
  const [progress, setProgress] = React.useState(initialBackupTestProgress);
  return (
    <>
      <ToolRow
        title="Create a key backup"
        description="Download a password-protected NIP-49 copy of your identity key."
        action="Create backup"
        open={createOpen}
        onToggle={() => setCreateOpen((v) => !v)}
        testId="profile-encrypted-backup-row"
      >
        <EncryptedBackupCreator guidedTest={false} variant="boxed" />
        <p className="mt-3 text-xs leading-5 text-muted-foreground">
          Keep the file private and save its password somewhere safe. Buzz
          cannot reset it.
        </p>
      </ToolRow>
      <div className="border-t border-border/60" />
      <ToolRow
        title="Test a key backup"
        description="Verify any NIP-49 backup and see which public identity it belongs to."
        action="Test backup"
        open={testOpen}
        onToggle={() => setTestOpen((v) => !v)}
        testId="profile-backup-test-row"
      >
        <BackupTestFlow
          variant="boxed"
          progress={progress}
          onProgressChange={setProgress}
        />
      </ToolRow>
    </>
  );
}
