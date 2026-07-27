import * as React from "react";
import { toast } from "sonner";

import { NsecMaskedDisplay } from "@/features/onboarding/ui/NsecMaskedDisplay";
import { getNsec, signOut } from "@/shared/api/tauriIdentity";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

/**
 * The exact phrase the user must type before the destructive sign-out button
 * unlocks. Kept lowercase; the comparison trims and lowercases input so a
 * stray capital or trailing space does not trip people up — the friction is
 * deliberate typing, not case sensitivity.
 */
export const SIGNOUT_CONFIRM_PHRASE = "wipe all my data";

/**
 * Sign-out card + destructive confirmation flow.
 *
 * Signing out wipes the identity key and all local data, so the confirm
 * dialog gates the delete button behind two explicit steps:
 *
 * 1. Back up the key — the nsec is shown inline (masked, with reveal/copy);
 *    the "I have saved my private key" checkbox unlocks only after the user
 *    actually reveals or copies the key.
 * 2. Typed confirmation — the user must type the exact phrase
 *    "wipe all my data".
 *
 * Only when both gates pass does "Delete My Data" become clickable.
 */
export function SignOutSection() {
  const [isOpen, setIsOpen] = React.useState(false);
  const [isPending, setIsPending] = React.useState(false);

  // Backup gate.
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [nsecError, setNsecError] = React.useState<string | null>(null);
  const [isNsecLoading, setIsNsecLoading] = React.useState(false);
  const [hasInteractedWithKey, setHasInteractedWithKey] = React.useState(false);
  const [hasConfirmedBackup, setHasConfirmedBackup] = React.useState(false);
  // Guards against a late-resolving getNsec() repopulating state after the
  // dialog closes.
  const fetchCancelledRef = React.useRef(false);

  // Typed-confirmation gate.
  const [confirmText, setConfirmText] = React.useState("");
  const isPhraseConfirmed =
    confirmText.trim().toLowerCase() === SIGNOUT_CONFIRM_PHRASE;

  // The backup checkbox unlocks after real interaction with the key
  // (reveal or copy). If the key cannot be loaded at all there is nothing to
  // interact with — let the user proceed past the backup step rather than
  // locking them out of sign-out entirely.
  const isBackupGateSatisfied = hasConfirmedBackup;
  const canConfirmBackup = hasInteractedWithKey || nsecError !== null;
  const canDelete = isBackupGateSatisfied && isPhraseConfirmed && !isPending;

  function resetDialogState() {
    fetchCancelledRef.current = true;
    setNsec(null);
    setNsecError(null);
    setIsNsecLoading(false);
    setHasInteractedWithKey(false);
    setHasConfirmedBackup(false);
    setConfirmText("");
  }

  React.useEffect(() => {
    return () => {
      fetchCancelledRef.current = true;
      setNsec(null);
    };
  }, []);

  async function openDialog() {
    setIsOpen(true);
    fetchCancelledRef.current = false;
    setIsNsecLoading(true);
    setNsecError(null);
    try {
      const value = await getNsec();
      if (!fetchCancelledRef.current) setNsec(value);
    } catch (err) {
      if (!fetchCancelledRef.current)
        setNsecError(
          err instanceof Error
            ? err.message
            : "Failed to retrieve private key.",
        );
    } finally {
      if (!fetchCancelledRef.current) setIsNsecLoading(false);
    }
  }

  function handleSignOut() {
    setIsPending(true);
    // Keep the pending state if signOut() resolves before restart.
    signOut()
      .then(() => {
        // Clear web storage for this origin on the success path only. This
        // covers dev builds where the Rust webview wipe targets the
        // .app-bundle WebKit dir (missing in `tauri dev`), preventing stale
        // community config from vouching for the fresh key on next boot. In
        // production the Rust wipe already handles this; the clear here is
        // redundant but harmless. The restart may race this clear — that is
        // acceptable; Fix A (pubkey-scoped heuristic) is the correctness
        // gate.
        window.localStorage.clear();
        window.sessionStorage.clear();
      })
      .catch((err: unknown) => {
        setIsPending(false);
        setIsOpen(false);
        resetDialogState();
        toast.error(err instanceof Error ? err.message : "Sign out failed.");
      });
  }

  return (
    <div
      className="mt-8 border-t border-border/60 pb-6 pt-5"
      data-testid="settings-signout"
    >
      <div className="flex items-center justify-between gap-4 px-1">
        <div className="min-w-0 space-y-1">
          <h2 className="text-lg font-semibold tracking-tight">Sign out</h2>
          <p className="text-sm text-muted-foreground">
            Removes your identity key and all local app data from this device.
            Back up your private key (nsec) first — this cannot be undone.
          </p>
        </div>
        <Button
          className="shrink-0"
          data-testid="signout-open-dialog"
          disabled={isPending}
          onClick={() => void openDialog()}
          type="button"
          variant="destructive"
        >
          {isPending ? (
            <Spinner aria-label="Signing out" className="h-4 w-4 border-2" />
          ) : null}
          {isPending ? "Signing out…" : "Sign Out"}
        </Button>
      </div>
      <AlertDialog
        onOpenChange={(open) => {
          if (!open && !isPending) {
            setIsOpen(false);
            resetDialogState();
          }
        }}
        open={isOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Sign out and wipe all data?</AlertDialogTitle>
            <AlertDialogDescription>
              This will delete your identity key, all agent settings, and cached
              data from this device, then relaunch Buzz into first-run setup.
              This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <div className="space-y-3">
            <p className="text-sm font-medium">
              1. Back up your private key (nsec)
            </p>
            {isNsecLoading ? (
              <p className="text-sm text-muted-foreground">Loading…</p>
            ) : nsecError ? (
              <p
                className="text-sm text-destructive"
                data-testid="signout-nsec-error"
              >
                {nsecError}
              </p>
            ) : nsec ? (
              <NsecMaskedDisplay
                nsec={nsec}
                onKeyInteraction={() => setHasInteractedWithKey(true)}
              />
            ) : null}
            <label
              className="flex cursor-pointer items-start gap-2.5 text-sm has-[button:disabled]:cursor-not-allowed has-[button:disabled]:opacity-60"
              data-testid="signout-backup-confirm-label"
              htmlFor="signout-backup-confirm"
            >
              <Checkbox
                checked={hasConfirmedBackup}
                className="mt-0.5"
                data-testid="signout-backup-confirm"
                disabled={!canConfirmBackup || isPending}
                id="signout-backup-confirm"
                onCheckedChange={(checked) =>
                  setHasConfirmedBackup(checked === true)
                }
              />
              <span>
                I have saved my private key somewhere safe.
                {!canConfirmBackup ? (
                  <span className="block text-xs text-muted-foreground">
                    Reveal or copy the key above first.
                  </span>
                ) : null}
              </span>
            </label>
          </div>

          <div className="space-y-2">
            <label
              className="text-sm font-medium"
              htmlFor="signout-confirm-phrase"
            >
              2. Type{" "}
              <span className="font-semibold">"{SIGNOUT_CONFIRM_PHRASE}"</span>{" "}
              to confirm
            </label>
            <Input
              autoComplete="off"
              data-testid="signout-confirm-phrase"
              disabled={isPending}
              id="signout-confirm-phrase"
              onChange={(event) => setConfirmText(event.target.value)}
              placeholder={SIGNOUT_CONFIRM_PHRASE}
              spellCheck={false}
              value={confirmText}
            />
          </div>

          <AlertDialogFooter>
            <AlertDialogCancel disabled={isPending}>Cancel</AlertDialogCancel>
            {/* A plain Button, not AlertDialogAction: Radix's Action closes
                the dialog on click, which would drop the pending state while
                the wipe + restart is still in flight. */}
            <Button
              data-testid="signout-confirm"
              disabled={!canDelete}
              onClick={handleSignOut}
              type="button"
              variant="destructive"
            >
              {isPending ? (
                <Spinner
                  aria-label="Signing out"
                  className="h-4 w-4 border-2"
                />
              ) : null}
              {isPending ? "Signing out…" : "Delete My Data"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
