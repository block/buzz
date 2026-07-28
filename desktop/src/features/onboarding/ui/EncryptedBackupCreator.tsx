import { AlertTriangle, RefreshCw } from "lucide-react";
import * as React from "react";

import {
  createNcryptsecBackup,
  generateBackupPassphrase,
  saveNcryptsecCopy,
} from "@/shared/api/tauriIdentity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import {
  createDisabled,
  customPassphraseIssue,
  encryptedBackupReducer,
  initialEncryptedBackupState,
  MIN_CUSTOM_PASSPHRASE_LEN,
  effectivePassphrase,
} from "../lib/encryptedBackup";
import { NsecMaskedDisplay } from "./NsecMaskedDisplay";

type EncryptedBackupCreatorProps = {
  /** "spotlight" is the onboarding treatment; "boxed" fits settings cards. */
  variant?: "spotlight" | "boxed";
  /** Fired only after the portable Keycase has been saved successfully. */
  onSaved?: (path: string) => void;
};

/**
 * Password-first Keycase creation flow shared by onboarding and Settings.
 * The raw private key never enters this component. Rust creates the NIP-49
 * payload locally, then the native save dialog produces the user-owned file.
 */
export function EncryptedBackupCreator({
  variant = "spotlight",
  onSaved,
}: EncryptedBackupCreatorProps) {
  const [state, dispatch] = React.useReducer(
    encryptedBackupReducer,
    initialEncryptedBackupState,
  );
  const [savedPath, setSavedPath] = React.useState<string | null>(null);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [isSaving, setIsSaving] = React.useState(false);
  const mountedRef = React.useRef(true);

  const generate = React.useCallback(async () => {
    try {
      const passphrase = await generateBackupPassphrase();
      if (mountedRef.current)
        dispatch({ type: "passphrase-generated", passphrase });
    } catch (err) {
      if (mountedRef.current)
        dispatch({
          type: "passphrase-generate-failed",
          message:
            err instanceof Error
              ? err.message
              : "Failed to generate a Keycase password.",
        });
    }
  }, []);

  React.useEffect(() => {
    mountedRef.current = true;
    void generate();
    return () => {
      mountedRef.current = false;
    };
  }, [generate]);

  const handleCreate = React.useCallback(async () => {
    const passphrase = effectivePassphrase(state);
    if (!passphrase || state.isCreating) return;
    dispatch({ type: "create-started" });
    try {
      const ncryptsec = await createNcryptsecBackup(passphrase);
      if (!mountedRef.current) return;
      dispatch({ type: "create-succeeded", ncryptsec });
      setIsSaving(true);
      setSaveError(null);
      try {
        const path = await saveNcryptsecCopy(ncryptsec);
        if (mountedRef.current && path) {
          setSavedPath(path);
          onSaved?.(path);
        }
      } catch (err) {
        if (mountedRef.current)
          setSaveError(
            err instanceof Error ? err.message : "Failed to save Keycase.",
          );
      } finally {
        if (mountedRef.current) setIsSaving(false);
      }
    } catch (err) {
      if (mountedRef.current)
        dispatch({
          type: "create-failed",
          message:
            err instanceof Error ? err.message : "Failed to create Keycase.",
        });
    }
  }, [onSaved, state]);

  const handleSaveCopy = React.useCallback(async () => {
    if (!state.ncryptsec || isSaving) return;
    setIsSaving(true);
    setSaveError(null);
    try {
      const path = await saveNcryptsecCopy(state.ncryptsec);
      if (mountedRef.current && path) {
        setSavedPath(path);
        onSaved?.(path);
      }
    } catch (err) {
      if (mountedRef.current)
        setSaveError(
          err instanceof Error ? err.message : "Failed to save Keycase.",
        );
    } finally {
      if (mountedRef.current) setIsSaving(false);
    }
  }, [isSaving, onSaved, state.ncryptsec]);

  const isSpotlight = variant === "spotlight";
  const customIssue = customPassphraseIssue(
    state.customPassphrase,
    state.customConfirm,
  );

  if (state.ncryptsec) {
    return (
      <div className="space-y-4" data-testid="encrypted-backup-result">
        <NsecMaskedDisplay
          kind="ncryptsec"
          nsec={state.ncryptsec}
          variant={isSpotlight ? "bare" : "boxed"}
        />
        <div className="flex flex-wrap items-center justify-center gap-3">
          <Button
            className="h-8 gap-1.5 text-sm"
            data-testid="encrypted-backup-save-copy"
            disabled={isSaving}
            onClick={() => void handleSaveCopy()}
            size="sm"
            type="button"
            variant="outline"
          >
            {isSaving ? <Spinner className="h-3.5 w-3.5 border-2" /> : null}
            Save Keycase…
          </Button>
          {savedPath ? (
            <p
              className="text-xs text-muted-foreground"
              data-testid="encrypted-backup-saved-path"
            >
              Keycase saved to {savedPath}
            </p>
          ) : null}
        </div>
        {saveError ? (
          <p className="text-center text-sm text-destructive">{saveError}</p>
        ) : null}
        <p className="text-center text-xs leading-5 text-muted-foreground">
          Keep this Keycase private. You need both the file and its password to
          restore your identity. Buzz cannot reset the password.
        </p>
      </div>
    );
  }

  return (
    <div
      className={cn("mx-auto w-full max-w-[500px] space-y-4 text-left")}
      data-testid="encrypted-backup-creator"
    >
      {state.mode === "generated" ? (
        <div className="space-y-3">
          {state.generatedPassphrase ? (
            <div className="rounded-xl bg-white/50 px-6 py-5 text-center">
              <p
                className="font-mono text-lg leading-8 text-foreground"
                data-testid="backup-passphrase-generated"
              >
                {state.generatedPassphrase}
              </p>
            </div>
          ) : state.generateError ? (
            <div
              className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
              data-testid="backup-passphrase-generate-error"
            >
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                Could not generate a Keycase password: {state.generateError}
              </span>
            </div>
          ) : (
            <div className="flex items-center justify-center gap-2 py-4 text-sm text-foreground/70">
              <Spinner className="h-4 w-4 border-2" />
              Generating a password…
            </div>
          )}
          <div className="flex items-center justify-center gap-3">
            <Button
              className="h-8 gap-1.5 text-sm"
              data-testid="backup-passphrase-regenerate"
              onClick={() => void generate()}
              size="sm"
              type="button"
              variant="outline"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              New generated password
            </Button>
            <Button
              className="h-8 text-sm text-muted-foreground hover:text-accent-foreground"
              data-testid="backup-passphrase-choose-own"
              onClick={() => dispatch({ type: "set-mode", mode: "custom" })}
              size="sm"
              type="button"
              variant="ghost"
            >
              Choose my own
            </Button>
          </div>
          <p className="text-center text-xs leading-5 text-muted-foreground">
            Save this generated passphrase as your Keycase password. Store it
            separately from the private Keycase file.
          </p>
        </div>
      ) : (
        <div className="space-y-3">
          <div className="space-y-2">
            <Input
              aria-label="Keycase password"
              autoComplete="new-password"
              className="h-10 bg-background"
              data-testid="backup-passphrase-custom"
              onChange={(event) =>
                dispatch({
                  type: "set-custom-passphrase",
                  value: event.target.value,
                })
              }
              placeholder={`Password (min ${MIN_CUSTOM_PASSPHRASE_LEN} characters)`}
              type="password"
              value={state.customPassphrase}
            />
            <Input
              aria-label="Confirm Keycase password"
              autoComplete="new-password"
              className="h-10 bg-background"
              data-testid="backup-passphrase-confirm"
              onChange={(event) =>
                dispatch({
                  type: "set-custom-confirm",
                  value: event.target.value,
                })
              }
              placeholder="Confirm password"
              type="password"
              value={state.customConfirm}
            />
          </div>
          {customIssue ? (
            <p
              className="text-sm text-muted-foreground"
              data-testid="backup-passphrase-issue"
            >
              {customIssue}
            </p>
          ) : null}
          <div className="flex items-center justify-center">
            <Button
              className="h-8 text-sm text-muted-foreground hover:text-accent-foreground"
              data-testid="backup-passphrase-use-generated"
              onClick={() => dispatch({ type: "set-mode", mode: "generated" })}
              size="sm"
              type="button"
              variant="ghost"
            >
              Use a generated password
            </Button>
          </div>
          <p className="text-center text-xs leading-5 text-muted-foreground">
            Your password protects the Keycase. Buzz cannot reset it if lost.
          </p>
        </div>
      )}

      {state.createError ? (
        <p
          className="text-center text-sm text-destructive"
          data-testid="encrypted-backup-create-error"
        >
          {state.createError}
        </p>
      ) : null}

      <div className="flex justify-center">
        <Button
          className="h-9 rounded-full px-6"
          data-testid="encrypted-backup-create"
          disabled={createDisabled(state)}
          onClick={() => void handleCreate()}
          type="button"
        >
          {state.isCreating ? (
            <>
              <Spinner className="h-4 w-4 border-2" />
              Creating Keycase… this takes a couple of seconds
            </>
          ) : (
            "Create Keycase"
          )}
        </Button>
      </div>
    </div>
  );
}
