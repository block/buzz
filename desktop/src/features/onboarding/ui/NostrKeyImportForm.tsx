import * as React from "react";
import { Check, KeyRound } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { nsecToNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

const NOSTR_KEY_FILE_MAX_BYTES = 1024;

type NostrKeyImportFormProps = {
  backLabel?: string;
  disabled?: boolean;
  errorMessage?: string | null;
  onBack: () => void;
  onImport: (nsec: string) => Promise<void>;
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
  variant = "default",
}: NostrKeyImportFormProps) {
  const [nsecInput, setNsecInput] = React.useState("");
  const [isImporting, setIsImporting] = React.useState(false);
  const [importError, setImportError] = React.useState<string | null>(null);
  const [isDragging, setIsDragging] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const previewNpub = React.useMemo(() => nsecToNpub(nsecInput), [nsecInput]);
  const trimmedInput = nsecInput.trim();
  const hasInput = trimmedInput.length > 0;
  const isValid = previewNpub !== null;
  const isInteractionDisabled = disabled || isImporting;
  const showInvalidHint = hasInput && !isValid && trimmedInput.length >= 5;
  const errorMessage = importError ?? externalErrorMessage;

  React.useLayoutEffect(() => {
    inputRef.current?.focus();
  }, []);

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
        "That file is too large to be a key. Drop a .key file or paste your nsec.",
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
    if (!previewNpub) {
      setImportError(
        "That doesn't look like a valid nsec. Paste an nsec1 key.",
      );
      return;
    }

    setIsImporting(true);
    setImportError(null);

    try {
      await onImport(trimmedInput);
    } catch (error) {
      setImportError(
        error instanceof Error ? error.message : "Couldn't import this key.",
      );
    } finally {
      setIsImporting(false);
    }
  }, [onImport, previewNpub, trimmedInput]);

  return (
    <form
      className="mt-8 flex w-full flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        void handleSubmit();
      }}
    >
      <div className="space-y-1.5 text-left">
        {variant === "spotlight" ? null : (
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="nostr-private-key"
          >
            Private key
          </label>
        )}
        <Input
          autoComplete="off"
          autoCorrect="off"
          className={
            variant === "spotlight"
              ? "h-16 rounded-2xl border-0 bg-white/85 text-center font-mono !text-xl shadow-[0_0_70px_45px_rgba(255,255,255,0.85)] placeholder:text-foreground/30 focus-visible:ring-0"
              : "h-10 bg-background"
          }
          data-testid="nostr-import-nsec-input"
          id="nostr-private-key"
          onChange={(event) => {
            setNsecInput(event.target.value);
            setImportError(null);
          }}
          placeholder={
            variant === "spotlight" ? "Enter your key here" : "nsec1..."
          }
          ref={inputRef}
          spellCheck={false}
          type="password"
          value={nsecInput}
        />
      </div>

      {variant === "spotlight" ? null : (
        <>
          <input
            accept=".key,text/plain"
            className="sr-only"
            disabled={isInteractionDisabled}
            onChange={(event) => {
              void handleFiles(event.currentTarget.files);
              event.currentTarget.value = "";
            }}
            ref={fileInputRef}
            tabIndex={-1}
            type="file"
          />

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
        </>
      )}

      {previewNpub ? (
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
      ) : null}

      {showInvalidHint && !errorMessage ? (
        <p className="text-xs text-muted-foreground">
          Waiting for a valid nsec1 key.
        </p>
      ) : null}

      {errorMessage ? (
        <p className="text-center text-sm text-destructive">{errorMessage}</p>
      ) : null}

      <div
        className={
          variant === "spotlight"
            ? "mt-4 flex w-full flex-col items-center gap-3"
            : "flex w-full flex-col gap-3 pt-1"
        }
      >
        <Button
          className={
            variant === "spotlight" ? "h-10 rounded-full px-8" : "h-10 w-full"
          }
          data-testid="nostr-import-submit"
          disabled={!isValid || isInteractionDisabled}
          type="submit"
        >
          {isImporting ? (
            <Spinner aria-label="Importing key" className="h-4 w-4 border-2" />
          ) : variant === "spotlight" ? (
            "Next"
          ) : (
            "Continue with this key"
          )}
        </Button>

        <Button
          className={
            variant === "spotlight"
              ? "h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
              : "h-10 w-full text-muted-foreground hover:text-accent-foreground"
          }
          disabled={isImporting}
          onClick={onBack}
          type="button"
          variant="ghost"
        >
          {backLabel}
        </Button>
      </div>
    </form>
  );
}
