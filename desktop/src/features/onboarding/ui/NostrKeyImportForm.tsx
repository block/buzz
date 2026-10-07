import * as React from "react";
import { Check, Eye, EyeOff, FileKey2, KeyRound } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { nsecToNpub } from "@/shared/lib/nostrUtils";
import {
  classifyKeyImportInput,
  isPlausibleNcryptsec,
  keyImportSubmitEnabled,
} from "../lib/keyImportInput";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import {
  BackupFileUnlockPreview,
  BackupPasswordTimeline,
} from "./BackupPasswordTimeline";
import { OnboardingFooter } from "./OnboardingFooter";

const NOSTR_KEY_FILE_MAX_BYTES = 1024;

export type NostrKeyImportStage = "key-entry" | "backup-password";

type NostrKeyImportFormProps = {
  backLabel?: string;
  disabled?: boolean;
  errorMessage?: string | null;
  onBack: () => void;
  onImport: (nsec: string, password?: string) => Promise<void>;
  /** Reports whether an import is in flight so host-owned navigation can be disabled. */
  onImportingChange?: (isImporting: boolean) => void;
  onStageChange?: (stage: NostrKeyImportStage) => void;
  /** Hide the inline back control when the host renders navigation elsewhere. */
  showBack?: boolean;
  /** Keep password-stage navigation out of the form when the host owns Back. */
  showPasswordStageBack?: boolean;
  /** Restrict this instance to selecting a backup file instead of typing a key. */
  mode?: "key" | "backup";
  /** Dialogs keep their actions inside the surface instead of the onboarding dock. */
  footerMode?: "onboarding" | "inline";
  /** "spotlight" is the first-launch treatment: glowy centered input, no drop zone, pill buttons. */
  variant?: "default" | "spotlight";
};

/**
 * Paste-or-drop nsec import form with a live npub preview.
 *
 * Shared between the first-run welcome flow (no community yet) and the
 * onboarding profile flow (community exists, user wants to reuse an
 * existing key). The caller owns what happens after `onImport` resolves.
 */
export function NostrKeyImportForm({
  backLabel = "Back",
  disabled = false,
  errorMessage: externalErrorMessage = null,
  onBack,
  onImport,
  onImportingChange,
  onStageChange,
  showBack = true,
  showPasswordStageBack = true,
  mode = "key",
  footerMode = "onboarding",
  variant = "default",
}: NostrKeyImportFormProps) {
  const [nsecInput, setNsecInput] = React.useState("");
  const [passphrase, setPassphrase] = React.useState("");
  const [isImporting, setIsImporting] = React.useState(false);
  const importInFlightRef = React.useRef(false);
  const [importError, setImportError] = React.useState<string | null>(null);
  const [isDragging, setIsDragging] = React.useState(false);
  const dragDepthRef = React.useRef(0);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const passphraseInputRef = React.useRef<HTMLInputElement | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const previewNpub = React.useMemo(() => nsecToNpub(nsecInput), [nsecInput]);
  const trimmedInput = nsecInput.trim();
  const hasInput = trimmedInput.length > 0;
  const inputKind = classifyKeyImportInput(nsecInput);
  const isEncryptedInput = inputKind === "ncryptsec";
  const isPasswordStage = isPlausibleNcryptsec(nsecInput);

  // Masked-by-default must re-assert whenever the field empties: a sticky
  // reveal from a previous key must never apply to newly pasted content the
  // user hasn't chosen to expose.
  React.useEffect(() => {
    if (!hasInput) {
      setIsRevealed(false);
    }
  }, [hasInput]);
  // A stale passphrase must never ride along when the input stops being an
  // encrypted backup (cleared field, or replaced with a raw nsec).
  React.useEffect(() => {
    if (!isPasswordStage) {
      setPassphrase("");
    }
  }, [isPasswordStage]);
  const isValid = keyImportSubmitEnabled(nsecInput, passphrase);
  const isInteractionDisabled = disabled || isImporting;
  const showInvalidHint =
    hasInput &&
    !isPasswordStage &&
    previewNpub === null &&
    trimmedInput.length >= 5;
  const errorMessage = importError ?? externalErrorMessage;
  const Footer = footerMode === "inline" ? "div" : OnboardingFooter;

  React.useLayoutEffect(() => {
    if (isPasswordStage) {
      passphraseInputRef.current?.focus();
    } else {
      inputRef.current?.focus();
    }
  }, [isPasswordStage]);

  React.useEffect(() => {
    onStageChange?.(isPasswordStage ? "backup-password" : "key-entry");
  }, [isPasswordStage, onStageChange]);

  React.useEffect(() => {
    if (mode !== "backup" || isPasswordStage || isInteractionDisabled) {
      dragDepthRef.current = 0;
      setIsDragging(false);
      return;
    }

    const handleDragEnter = (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes("Files")) return;
      dragDepthRef.current += 1;
      setIsDragging(true);
    };
    const handleDragLeave = () => {
      dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
      if (dragDepthRef.current === 0) setIsDragging(false);
    };
    const handleDragEnd = () => {
      dragDepthRef.current = 0;
      setIsDragging(false);
    };

    window.addEventListener("dragenter", handleDragEnter);
    window.addEventListener("dragleave", handleDragLeave);
    window.addEventListener("drop", handleDragEnd);
    window.addEventListener("dragend", handleDragEnd);
    return () => {
      window.removeEventListener("dragenter", handleDragEnter);
      window.removeEventListener("dragleave", handleDragLeave);
      window.removeEventListener("drop", handleDragEnd);
      window.removeEventListener("dragend", handleDragEnd);
    };
  }, [isInteractionDisabled, isPasswordStage, mode]);

  const openFilePicker = React.useCallback(() => {
    if (isInteractionDisabled) {
      return;
    }

    fileInputRef.current?.click();
  }, [isInteractionDisabled]);

  const handleFiles = React.useCallback(async (files: FileList | null) => {
    const file = files?.[0];
    if (!file) {
      return;
    }

    if (file.size > NOSTR_KEY_FILE_MAX_BYTES) {
      setImportError(
        "That file is too large to be a key backup or private key. Choose another file.",
      );
      return;
    }

    try {
      const text = await file.text();
      const firstLine =
        text.split(/\r?\n/).find((line) => line.trim().length > 0) ?? "";
      setNsecInput(firstLine.trim());
      setImportError(null);
    } catch (error) {
      setImportError(
        error instanceof Error ? error.message : "Couldn't read that file.",
      );
    }
  }, []);

  const handleSubmit = React.useCallback(async () => {
    // Guard here, not just on the submit button: the button now lives in the
    // portaled footer as type="button", so the single-field form still submits
    // on Enter. Without this, pressing Enter during an in-flight import fires a
    // second concurrent onImport (double keyring write). A ref closes the
    // same-tick gap before React commits `isImporting`.
    if (isInteractionDisabled || importInFlightRef.current) {
      return;
    }

    if (!isValid) {
      setImportError(
        isPasswordStage
          ? "Enter the password for this key backup."
          : isEncryptedInput
            ? "That doesn't look like a complete ncryptsec backup."
            : "That doesn't look like a valid nsec. Paste an nsec1 key.",
      );
      return;
    }

    importInFlightRef.current = true;
    onImportingChange?.(true);
    setIsImporting(true);
    setImportError(null);

    try {
      await onImport(trimmedInput, isPasswordStage ? passphrase : undefined);
    } catch (error) {
      setImportError(
        error instanceof Error ? error.message : "Couldn't import this key.",
      );
    } finally {
      importInFlightRef.current = false;
      onImportingChange?.(false);
      setIsImporting(false);
    }
  }, [
    isEncryptedInput,
    isInteractionDisabled,
    isPasswordStage,
    isValid,
    onImport,
    onImportingChange,
    passphrase,
    trimmedInput,
  ]);

  const handleBack = React.useCallback(() => {
    if (!isPasswordStage) {
      onBack();
      return;
    }

    setNsecInput("");
    setPassphrase("");
    setImportError(null);
    setIsRevealed(false);
    onStageChange?.("key-entry");
  }, [isPasswordStage, onBack, onStageChange]);

  return (
    <form
      className="mt-8 flex w-full flex-col gap-4"
      onDragOver={(event) => {
        if (mode !== "backup" || isPasswordStage) return;
        event.preventDefault();
        if (!isInteractionDisabled) {
          event.dataTransfer.dropEffect = "copy";
        }
      }}
      onDrop={(event) => {
        if (mode !== "backup" || isPasswordStage) return;
        event.preventDefault();
        setIsDragging(false);
        if (!isInteractionDisabled) {
          void handleFiles(event.dataTransfer.files);
        }
      }}
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      {!isPasswordStage && mode === "key" ? (
        <div className="space-y-1.5 text-left">
          <label
            className={cn(
              "text-sm font-medium text-foreground",
              variant === "spotlight" && "sr-only",
            )}
            htmlFor="nostr-private-key"
          >
            Private key
          </label>
          {variant === "spotlight" ? (
            <Card
              className="w-full px-8 py-12"
              data-testid="nostr-import-card"
              variant="textured"
            >
              <div className="relative w-full">
                <Input
                  autoComplete="off"
                  autoCorrect="off"
                  // Symmetric px reserves the absolutely positioned toggle's
                  // footprint on BOTH sides, so the centered key text never
                  // runs under the eye control and stays optically centered.
                  className="h-[3.6875rem] rounded-none border-0 bg-transparent px-10 text-center font-mono !text-4xl text-[color:var(--buzz-onboarding-backup-ink)] shadow-none placeholder:text-foreground/30 focus-visible:ring-0"
                  data-testid="nostr-import-nsec-input"
                  id="nostr-private-key"
                  onChange={(event) => {
                    setNsecInput(event.target.value);
                    setImportError(null);
                  }}
                  placeholder="Enter your key here"
                  ref={inputRef}
                  spellCheck={false}
                  type={isRevealed ? "text" : "password"}
                  value={nsecInput}
                />
                {/* Absolutely positioned so appearing/disappearing never resizes
                  the input or shifts its centered text; fades with hasInput. */}
                <Button
                  aria-hidden={!hasInput}
                  aria-label={
                    isRevealed ? "Hide private key" : "Reveal private key"
                  }
                  className={cn(
                    "absolute right-8 top-1/2 h-10 w-10 -translate-y-1/2 text-muted-foreground transition-opacity duration-300 hover:bg-foreground/10 hover:text-foreground motion-reduce:transition-none",
                    hasInput ? "opacity-100" : "pointer-events-none opacity-0",
                  )}
                  data-testid="nostr-import-reveal-toggle"
                  onClick={() => setIsRevealed((current) => !current)}
                  size="icon"
                  tabIndex={hasInput ? 0 : -1}
                  type="button"
                  variant="ghost"
                >
                  {isRevealed ? (
                    <EyeOff aria-hidden="true" className="h-6 w-6" />
                  ) : (
                    <Eye aria-hidden="true" className="h-6 w-6" />
                  )}
                </Button>
              </div>
            </Card>
          ) : (
            <Input
              autoComplete="off"
              autoCorrect="off"
              className="h-10 bg-background"
              data-testid="nostr-import-nsec-input"
              id="nostr-private-key"
              onChange={(event) => {
                setNsecInput(event.target.value);
                setImportError(null);
              }}
              placeholder="nsec1..."
              ref={inputRef}
              spellCheck={false}
              type="password"
              value={nsecInput}
            />
          )}
        </div>
      ) : null}

      {/* Hidden file input shared by both variants: the default drop zone and
          the spotlight "Choose a backup file" button both open it. Accepts the
          .ncryptsec backups our own save flow emits alongside raw .key files. */}
      {mode === "backup" || variant !== "spotlight" ? (
        <input
          accept=".key,.ncryptsec,text/plain"
          className="sr-only"
          data-testid="nostr-import-file-input"
          disabled={isInteractionDisabled}
          id={
            mode === "backup"
              ? "nostr-import-backup-file-input"
              : "nostr-import-file-input"
          }
          onChange={(event) => {
            void handleFiles(event.currentTarget.files);
            event.currentTarget.value = "";
          }}
          ref={fileInputRef}
          tabIndex={-1}
          type="file"
        />
      ) : null}

      {!isPasswordStage && mode === "backup" ? (
        <>
          <div
            className="mx-auto flex h-[312px] w-full max-w-[500px] flex-col items-center justify-center gap-4 [@media(max-height:40rem)]:h-auto"
            data-testid="nostr-import-backup-file-section"
          >
            <Button
              className="h-9 rounded-full px-6"
              data-testid="nostr-import-backup-picker"
              disabled={isInteractionDisabled}
              onClick={openFilePicker}
              type="button"
            >
              <FileKey2 aria-hidden="true" className="mr-2 size-4" />
              Choose a backup file
            </Button>
            <BackupFileUnlockPreview />
          </div>
          {isDragging ? (
            <fieldset
              className="absolute inset-[var(--buzz-card-textured-safe-inset)] z-10 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/60 bg-background/80 backdrop-blur-sm"
              data-dragging="true"
              data-testid="nostr-import-backup-drop"
            >
              <span className="flex items-center gap-2 rounded-full bg-foreground px-4 py-2 text-sm font-semibold text-background shadow-sm ring-1 ring-background/15">
                <KeyRound aria-hidden="true" className="size-4" />
                <span>Drop your backup file here</span>
              </span>
            </fieldset>
          ) : null}
        </>
      ) : null}

      {!isPasswordStage && mode === "key" && variant !== "spotlight" ? (
        <button
          className={cn(
            "relative flex h-[120px] flex-col items-center justify-center gap-3 overflow-hidden rounded-xl border border-transparent bg-muted text-foreground transition-[background-color,border-color,box-shadow,color] duration-[250ms] ease-out hover:bg-muted/80 disabled:opacity-60",
            isDragging &&
              "border-primary bg-primary/10 text-primary ring-1 ring-primary/35 hover:bg-primary/10",
          )}
          data-dragging={isDragging ? "true" : undefined}
          data-testid="nostr-import-drop"
          disabled={isInteractionDisabled}
          onClick={openFilePicker}
          onDragEnter={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!isInteractionDisabled) {
              setIsDragging(true);
            }
          }}
          onDragLeave={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (
              event.currentTarget.contains(event.relatedTarget as Node | null)
            ) {
              return;
            }
            setIsDragging(false);
          }}
          onDragOver={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!isInteractionDisabled) {
              event.dataTransfer.dropEffect = "copy";
            }
          }}
          onDrop={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setIsDragging(false);
            if (isInteractionDisabled) {
              return;
            }
            void handleFiles(event.dataTransfer.files);
          }}
          type="button"
        >
          <span
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-0 rounded-[inherit] bg-primary/10 opacity-0 transition-opacity duration-[250ms] ease-out",
              isDragging && "opacity-100",
            )}
          />
          <KeyRound
            className={cn(
              "relative h-8 w-8 text-muted-foreground transition-colors duration-[250ms] ease-out",
              isDragging && "text-primary",
            )}
          />
          <span
            className={cn(
              "relative text-sm font-medium text-muted-foreground transition-colors duration-[250ms] ease-out",
              isDragging && "text-primary",
            )}
          >
            Drop a key here
          </span>
        </button>
      ) : null}

      {isPasswordStage ? (
        <div
          className="relative mx-auto w-full max-w-[500px] pb-32 pt-32 [@media(max-height:40rem)]:py-0"
          data-testid="nostr-import-passphrase-section"
        >
          <BackupPasswordTimeline mode="restore" />
          <label className="sr-only" htmlFor="nostr-import-passphrase">
            Backup password
          </label>
          <div className="relative z-10">
            <Input
              autoComplete="current-password"
              autoCorrect="off"
              className="h-14 rounded-2xl border-black/20 bg-white px-12 text-center font-mono text-lg text-black/80 shadow-none placeholder:text-black/55 focus-visible:ring-black/35"
              data-testid="nostr-import-passphrase"
              id="nostr-import-passphrase"
              onChange={(event) => {
                setPassphrase(event.target.value);
                setImportError(null);
              }}
              placeholder="Backup password"
              ref={passphraseInputRef}
              spellCheck={false}
              type={isRevealed ? "text" : "password"}
              value={passphrase}
            />
            <Button
              aria-label={isRevealed ? "Hide password" : "Reveal password"}
              className="absolute right-1 top-1/2 h-8 w-8 -translate-y-1/2 text-black/55 hover:bg-black/5 hover:text-black/80"
              data-testid="nostr-import-passphrase-reveal-toggle"
              disabled={isInteractionDisabled}
              onClick={() => setIsRevealed((current) => !current)}
              size="icon"
              type="button"
              variant="ghost"
            >
              {isRevealed ? (
                <EyeOff aria-hidden="true" className="h-4 w-4" />
              ) : (
                <Eye aria-hidden="true" className="h-4 w-4" />
              )}
            </Button>
          </div>
        </div>
      ) : null}

      {!isPasswordStage || errorMessage ? (
        <div
          className={cn(
            "min-h-8",
            variant === "spotlight" && "mt-6 text-center",
          )}
          data-testid="nostr-import-feedback"
        >
          {!isPasswordStage && previewNpub ? (
            variant === "spotlight" ? (
              // Spotlight uses the backup step's quiet caption language:
              // centered, unboxed, with the npub in the shared olive key ink.
              <div
                className="space-y-1 text-sm"
                data-testid="nostr-import-npub-preview"
              >
                <p className="flex items-center justify-center gap-1.5 text-foreground">
                  <Check aria-hidden="true" className="h-4 w-4 shrink-0" />
                  Nostr identity found
                </p>
                <p className="break-all font-mono text-[color:var(--buzz-onboarding-backup-ink)]">
                  {previewNpub}
                </p>
              </div>
            ) : (
              <div
                className="flex items-start gap-2 rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs"
                data-testid="nostr-import-npub-preview"
              >
                <Check className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
                <div className="min-w-0 space-y-0.5">
                  <p className="font-medium text-foreground">
                    This will use this Nostr identity:
                  </p>
                  <p className="break-all font-mono text-2xs text-muted-foreground">
                    {previewNpub}
                  </p>
                </div>
              </div>
            )
          ) : null}

          {showInvalidHint && !errorMessage ? (
            <p className="text-sm text-muted-foreground">
              {isEncryptedInput
                ? "Waiting for a complete ncryptsec backup"
                : "Waiting for a valid nsec1 key"}
            </p>
          ) : null}

          {errorMessage ? (
            <p className="text-center text-sm text-destructive">
              {errorMessage}
            </p>
          ) : null}
        </div>
      ) : null}

      <Footer
        className={
          footerMode === "inline"
            ? "mt-6 flex flex-col items-center gap-2"
            : undefined
        }
      >
        {mode === "key" || isPasswordStage ? (
          <Button
            className={
              // Only the spotlight (onboarding) treatment gets the docked pill CTA.
              // The default variant renders outside the onboarding footer provider
              // (e.g. KeyringLockedScreen) and must stay full-width to match its
              // sibling Back button.
              variant === "spotlight"
                ? ONBOARDING_PRIMARY_CTA_CLASS
                : "h-10 w-full"
            }
            data-testid="nostr-import-submit"
            disabled={!isValid || isInteractionDisabled}
            onClick={() => void handleSubmit()}
            type="button"
          >
            {isImporting ? (
              <Spinner
                aria-label="Importing key"
                className="h-4 w-4 border-2"
              />
            ) : variant === "spotlight" ? (
              "Next"
            ) : (
              "Continue with this key"
            )}
          </Button>
        ) : null}

        {showBack || (isPasswordStage && showPasswordStageBack) ? (
          <Button
            className={
              variant === "spotlight"
                ? ONBOARDING_SECONDARY_CTA_CLASS
                : "h-10 w-full text-muted-foreground hover:text-accent-foreground"
            }
            disabled={isImporting}
            onClick={handleBack}
            type="button"
            variant="ghost"
          >
            {isPasswordStage ? "Back" : backLabel}
          </Button>
        ) : null}
      </Footer>
    </form>
  );
}
