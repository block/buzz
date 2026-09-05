import { Check, Copy, Eye, EyeOff, FileDown, ShieldCheck } from "lucide-react";
import * as React from "react";

import {
  createTwoSkdBackup,
  type CreatedTwoSkdBackup,
  saveTwoSkdBackupCopy,
  saveTwoSkdRecoverySheet,
} from "@/shared/api/tauriIdentity";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

const MIN_PASSPHRASE_LEN = 12;

/** Create the smallest useful 2SKD recovery kit: one encrypted file plus a
 * separately-held recovery code. The raw identity key never reaches React. */
export function TwoSkdRecoveryCreator({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const [passphrase, setPassphrase] = React.useState("");
  const [revealed, setRevealed] = React.useState(false);
  const [created, setCreated] = React.useState<CreatedTwoSkdBackup | null>(
    null,
  );
  const [savedPath, setSavedPath] = React.useState<string | null>(null);
  const [recoverySheetPath, setRecoverySheetPath] = React.useState<
    string | null
  >(null);
  const [pending, setPending] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const copyTimerRef = React.useRef<number | null>(null);
  const requestRef = React.useRef(0);

  const reset = React.useCallback(() => {
    requestRef.current += 1;
    setPassphrase("");
    setRevealed(false);
    setCreated(null);
    setSavedPath(null);
    setRecoverySheetPath(null);
    setPending(false);
    setCopied(false);
    setError(null);
    if (copyTimerRef.current !== null) {
      window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = null;
    }
  }, []);

  React.useEffect(
    () => () => {
      if (copyTimerRef.current !== null)
        window.clearTimeout(copyTimerRef.current);
    },
    [],
  );

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) reset();
      onOpenChange(nextOpen);
    },
    [onOpenChange, reset],
  );

  const saveBackup = React.useCallback(
    (result: CreatedTwoSkdBackup) => saveTwoSkdBackupCopy(result.backup),
    [],
  );

  const create = React.useCallback(async () => {
    if ([...passphrase].length < MIN_PASSPHRASE_LEN || pending) return;
    const requestId = ++requestRef.current;
    setPending(true);
    setError(null);
    setRevealed(false);
    try {
      const result = await createTwoSkdBackup(passphrase);
      if (requestId !== requestRef.current) return;
      setPassphrase("");
      setCreated(result);
      const path = await saveBackup(result);
      if (requestId !== requestRef.current) return;
      if (path) setSavedPath(path);
    } catch (cause) {
      if (requestId !== requestRef.current) return;
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not create your recovery kit.",
      );
    } finally {
      if (requestId === requestRef.current) setPending(false);
    }
  }, [passphrase, pending, saveBackup]);

  const copyRecoveryCode = React.useCallback(async () => {
    if (!created) return;
    try {
      await writeTextToClipboard(created.recoverySecret);
      setCopied(true);
      if (copyTimerRef.current !== null)
        window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not copy the code.",
      );
    }
  }, [created]);

  const saveRecoverySheet = React.useCallback(async () => {
    if (!created || pending) return;
    const requestId = ++requestRef.current;
    setPending(true);
    setError(null);
    try {
      const path = await saveTwoSkdRecoverySheet(
        created.backup,
        created.recoverySecret,
      );
      if (requestId !== requestRef.current) return;
      if (path) setRecoverySheetPath(path);
    } catch (cause) {
      if (requestId !== requestRef.current) return;
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save the recovery sheet.",
      );
    } finally {
      if (requestId === requestRef.current) setPending(false);
    }
  }, [created, pending]);

  const retrySave = React.useCallback(async () => {
    if (!created || pending) return;
    const requestId = ++requestRef.current;
    setPending(true);
    setError(null);
    try {
      const path = await saveBackup(created);
      if (requestId !== requestRef.current) return;
      if (path) setSavedPath(path);
    } catch (cause) {
      if (requestId !== requestRef.current) return;
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save the encrypted backup.",
      );
    } finally {
      if (requestId === requestRef.current) setPending(false);
    }
  }, [created, pending, saveBackup]);

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-lg" data-testid="two-skd-dialog">
        <DialogHeader className="pr-8">
          <DialogTitle>Create a recovery kit</DialogTitle>
          <DialogDescription>
            Recovery requires the encrypted backup, your password, and a
            separate recovery code.
          </DialogDescription>
        </DialogHeader>

        {!created ? (
          <div className="space-y-4" data-testid="two-skd-password-step">
            <div className="relative">
              <Input
                aria-label="Recovery password"
                autoComplete="new-password"
                className="h-10 bg-background pr-10"
                disabled={pending}
                onChange={(event) => {
                  setPassphrase(event.target.value);
                  setError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void create();
                  }
                }}
                placeholder={`Password (min ${MIN_PASSPHRASE_LEN} characters)`}
                type={revealed ? "text" : "password"}
                value={passphrase}
              />
              <Button
                aria-label={revealed ? "Hide password" : "Reveal password"}
                className="absolute right-1 top-1/2 size-8 -translate-y-1/2 text-muted-foreground"
                disabled={pending}
                onClick={() => setRevealed((value) => !value)}
                size="icon"
                type="button"
                variant="ghost"
              >
                {revealed ? (
                  <EyeOff aria-hidden="true" className="size-4" />
                ) : (
                  <Eye aria-hidden="true" className="size-4" />
                )}
              </Button>
            </div>
            <p className="text-xs leading-5 text-muted-foreground">
              Buzz combines this password with a random code that never enters
              the encrypted backup. A stolen backup alone cannot be tested
              against guessed passwords.
            </p>
            <div className="flex justify-end">
              <Button
                className="rounded-full px-6"
                data-testid="two-skd-create"
                disabled={
                  [...passphrase].length < MIN_PASSPHRASE_LEN || pending
                }
                onClick={() => void create()}
                type="button"
              >
                {pending ? <Spinner className="size-4 border-2" /> : null}
                Create kit
              </Button>
            </div>
          </div>
        ) : savedPath ? (
          <div className="space-y-4" data-testid="two-skd-recovery-code-step">
            <div className="flex items-start gap-3 rounded-xl border border-primary/30 bg-primary/5 p-4">
              <ShieldCheck
                aria-hidden="true"
                className="mt-0.5 size-5 shrink-0 text-primary"
              />
              <div className="min-w-0 space-y-2">
                <p className="text-sm font-medium">Save this separately</p>
                <p
                  className="break-all font-mono text-sm leading-6"
                  data-testid="two-skd-recovery-code"
                >
                  {created.recoverySecret}
                </p>
              </div>
            </div>
            <p className="text-xs leading-5 text-muted-foreground">
              The encrypted backup was saved to {savedPath}. Put this code in a
              different place. The printable PDF includes a QR code and recovery
              instructions, but never your password or encrypted backup. Buzz
              cannot recreate it later.
            </p>
            {recoverySheetPath ? (
              <p
                className="rounded-lg bg-muted px-3 py-2 text-xs text-muted-foreground"
                data-testid="two-skd-recovery-sheet-path"
              >
                Recovery sheet saved to {recoverySheetPath}
              </p>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
              <Button
                data-testid="two-skd-copy-code"
                onClick={() => void copyRecoveryCode()}
                type="button"
                variant="secondary"
              >
                {copied ? (
                  <Check aria-hidden="true" className="size-4" />
                ) : (
                  <Copy aria-hidden="true" className="size-4" />
                )}
                {copied ? "Copied" : "Copy code"}
              </Button>
              <Button
                data-testid="two-skd-save-recovery-sheet"
                disabled={pending}
                onClick={() => void saveRecoverySheet()}
                type="button"
              >
                {pending ? (
                  <Spinner className="size-4 border-2" />
                ) : recoverySheetPath ? (
                  <Check aria-hidden="true" className="size-4" />
                ) : (
                  <FileDown aria-hidden="true" className="size-4" />
                )}
                {recoverySheetPath ? "Save another PDF" : "Save printable PDF"}
              </Button>
              <Button
                onClick={() => handleOpenChange(false)}
                type="button"
                variant="ghost"
              >
                Done
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-4" data-testid="two-skd-save-step">
            <p className="text-sm leading-6 text-muted-foreground">
              Save the encrypted backup before Buzz shows its separate recovery
              code.
            </p>
            <div className="flex justify-end">
              <Button
                disabled={pending}
                onClick={() => void retrySave()}
                type="button"
              >
                {pending ? <Spinner className="size-4 border-2" /> : null}
                Save encrypted backup
              </Button>
            </div>
          </div>
        )}

        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
