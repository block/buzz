import { Check, Copy, Eye, EyeOff, FileKey2, FileUp } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  verifyNcryptsecBackup,
  type BackupVerification,
} from "@/shared/api/tauriIdentity";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

export type BackupTestProgress = {
  stage: "drop" | "password" | "success";
  fileName: string | null;
  ncryptsec: string | null;
  result: BackupVerification | null;
};
export const initialBackupTestProgress: BackupTestProgress = {
  stage: "drop",
  fileName: null,
  ncryptsec: null,
  result: null,
};

type Props = {
  variant?: "spotlight" | "boxed";
  /** When supplied, only this exact just-created file can complete onboarding. */
  expectedNcryptsec?: string;
  onSaveCopy?: () => void;
  isSaving?: boolean;
  savedPath?: string | null;
  saveError?: string | null;
  progress: BackupTestProgress;
  onProgressChange: React.Dispatch<React.SetStateAction<BackupTestProgress>>;
  onVerified?: () => void;
};

export function BackupTestFlow({
  variant = "spotlight",
  expectedNcryptsec,
  onSaveCopy,
  isSaving = false,
  savedPath,
  saveError,
  progress,
  onProgressChange,
  onVerified,
}: Props) {
  const { stage, fileName, ncryptsec, result } = progress;
  const [attempt, setAttempt] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [verifying, setVerifying] = React.useState(false);
  const [revealed, setRevealed] = React.useState(false);
  const [dragging, setDragging] = React.useState(false);
  const fileRef = React.useRef<HTMLInputElement>(null);
  const mounted = React.useRef(true);
  const requestRef = React.useRef(0);
  React.useEffect(
    () => () => {
      mounted.current = false;
      requestRef.current += 1;
      setAttempt("");
    },
    [],
  );

  React.useEffect(() => {
    const enter = (event: DragEvent) => {
      if (event.dataTransfer?.types.includes("Files")) setDragging(true);
    };
    const leave = (event: DragEvent) => {
      if (event.clientX === 0 && event.clientY === 0) setDragging(false);
    };
    const end = () => setDragging(false);
    window.addEventListener("dragenter", enter);
    window.addEventListener("dragleave", leave);
    window.addEventListener("drop", end);
    window.addEventListener("dragend", end);
    return () => {
      window.removeEventListener("dragenter", enter);
      window.removeEventListener("dragleave", leave);
      window.removeEventListener("drop", end);
      window.removeEventListener("dragend", end);
    };
  }, []);

  async function choose(file: File) {
    try {
      const text = (await file.text()).trim();
      if (!text.toLowerCase().startsWith("ncryptsec1"))
        throw new Error("That doesn't look like a NIP-49 key backup.");
      if (expectedNcryptsec && text !== expectedNcryptsec.trim())
        throw new Error(
          "That's a key backup, but not the one you just downloaded.",
        );
      setError(null);
      setAttempt("");
      onProgressChange({
        stage: "password",
        fileName: file.name,
        ncryptsec: text,
        result: null,
      });
    } catch (err) {
      if (mounted.current)
        setError(
          err instanceof Error ? err.message : "Could not read that file.",
        );
    }
  }

  async function verify() {
    if (!ncryptsec || !attempt || verifying) return;
    const password = attempt;
    const id = ++requestRef.current;
    setVerifying(true);
    setError(null);
    setRevealed(false);
    setAttempt("");
    try {
      const verified = await verifyNcryptsecBackup(ncryptsec, password);
      if (!mounted.current || id !== requestRef.current) return;
      onProgressChange({ ...progress, stage: "success", result: verified });
      onVerified?.();
    } catch (err) {
      if (mounted.current && id === requestRef.current)
        setError(
          err instanceof Error ? err.message : "Could not verify this backup.",
        );
    } finally {
      if (mounted.current && id === requestRef.current) setVerifying(false);
    }
  }

  if (stage === "success" && result)
    return (
      <div
        className="space-y-4 py-3 text-center"
        data-testid="backup-test-success"
      >
        <div className="mx-auto flex size-14 items-center justify-center rounded-full bg-primary text-primary-foreground">
          <Check className="size-7" aria-hidden />
        </div>
        <div>
          <p className="text-lg font-medium">Valid backup</p>
          <p className="mt-1 text-sm text-muted-foreground">
            {result.matchesCurrentIdentity
              ? "Matches your current Buzz identity."
              : "Belongs to a different identity."}
          </p>
        </div>
        <div className="mx-auto flex max-w-md items-center gap-2 rounded-lg bg-muted px-3 py-2">
          <code
            className="min-w-0 flex-1 truncate text-xs"
            title={result.npub}
            data-testid="backup-test-npub"
          >
            {result.npub}
          </code>
          <Button
            aria-label="Copy backup identity"
            data-testid="backup-test-copy-npub"
            onClick={async () => {
              await writeTextToClipboard(result.npub);
              toast.success("Copied to clipboard");
            }}
            size="icon"
            variant="ghost"
          >
            <Copy className="size-4" />
          </Button>
        </div>
        {!expectedNcryptsec ? (
          <Button
            data-testid="backup-test-another"
            onClick={() => {
              setError(null);
              onProgressChange(initialBackupTestProgress);
            }}
            variant="ghost"
          >
            Test another backup
          </Button>
        ) : null}
      </div>
    );

  const spotlight = variant === "spotlight";
  return (
    <div
      className={cn(
        "mx-auto w-full space-y-4",
        spotlight ? "max-w-140" : "max-w-125",
      )}
      data-testid="backup-test-flow"
    >
      {stage === "drop" ? (
        <>
          <input
            ref={fileRef}
            className="sr-only"
            data-testid="backup-test-file-input"
            type="file"
            accept=".ncryptsec,text/plain"
            tabIndex={-1}
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) void choose(file);
            }}
          />
          <button
            type="button"
            data-testid="backup-test-dropzone"
            className={cn(
              "mx-auto flex items-center justify-center rounded-full bg-primary font-medium text-primary-foreground",
              spotlight ? "h-14 px-12" : "h-9 px-6 text-sm",
            )}
            onClick={() => fileRef.current?.click()}
          >
            Select a backup file
          </button>
          {dragging ? (
            <section
              aria-label="Drop your key backup here"
              className="absolute inset-2 z-10 mt-0! flex items-center justify-center rounded-2xl border-2 border-dashed border-primary bg-primary/10 backdrop-blur-sm"
              data-testid="backup-test-drop-overlay"
              onDragOver={(e) => e.preventDefault()}
              onDrop={(e) => {
                e.preventDefault();
                const f = e.dataTransfer.files?.[0];
                if (f) void choose(f);
              }}
            >
              <span className="flex items-center gap-2 rounded-full bg-foreground px-4 py-2 text-sm font-semibold text-background">
                <FileUp className="size-4" />
                Drop your backup file here
              </span>
            </section>
          ) : null}
          {onSaveCopy ? (
            <div className="text-center">
              <Button
                data-testid="encrypted-backup-save-copy"
                disabled={isSaving}
                onClick={onSaveCopy}
                variant="ghost"
              >
                {isSaving ? <Spinner className="mr-2 size-4 border-2" /> : null}
                Re-download backup
              </Button>
              {savedPath ? (
                <p
                  className="text-xs text-muted-foreground"
                  data-testid="encrypted-backup-saved-path"
                >
                  Saved to {savedPath}
                </p>
              ) : null}
            </div>
          ) : null}
        </>
      ) : (
        <>
          <div
            className="flex items-center justify-center gap-2 text-sm"
            data-testid="backup-test-file-accepted"
          >
            <FileKey2 className="size-4" />
            <span className="max-w-70 truncate font-mono text-xs">
              {fileName}
            </span>
            <Check className="size-4 text-primary" />
          </div>
          <p className="text-center text-sm text-muted-foreground">
            Enter its password, then verify it with real NIP-49 decryption.
          </p>
          <div className="relative">
            <Input
              aria-label="Backup password"
              autoComplete="off"
              data-testid="backup-test-password"
              disabled={verifying}
              type={revealed ? "text" : "password"}
              value={attempt}
              onChange={(e) => setAttempt(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void verify();
                }
              }}
              className="pr-10 font-mono"
            />
            <Button
              aria-label={revealed ? "Hide password" : "Reveal password"}
              data-testid="backup-test-password-reveal-toggle"
              disabled={verifying}
              onClick={() => setRevealed((v) => !v)}
              size="icon"
              type="button"
              variant="ghost"
              className="absolute right-1 top-1/2 -translate-y-1/2"
            >
              {revealed ? (
                <EyeOff className="size-4" />
              ) : (
                <Eye className="size-4" />
              )}
            </Button>
          </div>
          <div className="flex justify-center gap-2">
            <Button
              data-testid="backup-test-verify"
              disabled={!attempt || verifying}
              onClick={() => void verify()}
            >
              {verifying ? <Spinner className="mr-2 size-4 border-2" /> : null}
              Verify backup
            </Button>
            <Button
              data-testid="backup-test-use-different-file"
              disabled={verifying}
              onClick={() => {
                requestRef.current += 1;
                setAttempt("");
                setError(null);
                onProgressChange(initialBackupTestProgress);
              }}
              variant="ghost"
            >
              Use a different file
            </Button>
          </div>
        </>
      )}
      {error || saveError ? (
        <p
          role="alert"
          className="text-center text-sm text-destructive"
          data-testid="backup-test-error"
        >
          {error ?? saveError}
        </p>
      ) : null}
    </div>
  );
}
