import { AlertTriangle, Eye, EyeOff, RefreshCw } from "lucide-react";
import * as React from "react";
import { createPortal } from "react-dom";

import {
  createNcryptsecBackup,
  generateBackupPassphrase,
  saveNcryptsecCopy,
} from "@/shared/api/tauriIdentity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import {
  downloadDisabled,
  isEncrypting,
  passphraseIssue,
  pendingEncryptPassphrase,
  encryptedBackupReducer,
  initialEncryptedBackupState,
  MIN_PASSPHRASE_LEN,
} from "../lib/encryptedBackup";
import { NsecMaskedDisplay } from "./NsecMaskedDisplay";

/** Word-count bounds mirroring `key_backup.rs` (Rust clamps regardless). */
const MIN_GENERATED_WORDS = 3;
const MAX_GENERATED_WORDS = 10;
const DEFAULT_GENERATED_WORDS = 3;

const SEPARATOR_OPTIONS = [
  { label: "Spaces", value: " " },
  { label: "Hyphens", value: "-" },
  { label: "Periods", value: "." },
  { label: "Commas", value: "," },
] as const;

const DEFAULT_SEPARATOR = SEPARATOR_OPTIONS[0].value;

/**
 * Pause after the last keystroke before the background KDF starts, so typing
 * past the minimum length doesn't launch an encryption per character.
 */
const ENCRYPT_DEBOUNCE_MS = 400;

type EncryptedBackupCreatorProps = {
  /** "spotlight" is the onboarding treatment; "boxed" fits settings cards. */
  variant?: "spotlight" | "boxed";
  /**
   * When set, the "Download" button is portaled into this element
   * (e.g. the onboarding footer's primary slot) instead of rendering inline.
   */
  createButtonPortal?: HTMLElement | null;
  /** Extra classes for the "Download" button. */
  createButtonClassName?: string;
  /** Fired once the encrypted payload has been created (before saving). */
  onCreated?: () => void;
  /** Fired only after the encrypted key file has been saved successfully. */
  onSaved?: (path: string) => void;
};

/**
 * 1Password-style memorable-password generator popover with word-count and
 * separator fields, anchored to a refresh icon inset in the password field
 * (the anchor assumes a `relative` parent). The first click opens the
 * popover and generates; further clicks on the icon re-roll while the
 * popover stays open — only click-outside or Esc closes it. There is no
 * candidate preview: every generation writes the passphrase straight into
 * the parent's password field via `onGenerated`.
 */
function PassphraseGeneratorPopover({
  disabled = false,
  onGenerated,
}: {
  disabled?: boolean;
  onGenerated: (value: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const [words, setWords] = React.useState(DEFAULT_GENERATED_WORDS);
  const [separator, setSeparator] = React.useState<string>(DEFAULT_SEPARATOR);
  const [error, setError] = React.useState<string | null>(null);
  const anchorRef = React.useRef<HTMLButtonElement | null>(null);
  const mountedRef = React.useRef(true);
  // Read via a ref so `generate` stays reference-stable even though parents
  // pass an inline `onGenerated`. Otherwise each generated password would
  // re-render the parent, rebuild `generate`, and re-fire the open/controls
  // effect below — an infinite generate loop while the popover is open.
  const onGeneratedRef = React.useRef(onGenerated);

  React.useEffect(() => {
    onGeneratedRef.current = onGenerated;
  }, [onGenerated]);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const generate = React.useCallback(async (wordCount: number, sep: string) => {
    setError(null);
    try {
      const passphrase = await generateBackupPassphrase({
        words: wordCount,
        separator: sep,
      });
      if (mountedRef.current) onGeneratedRef.current(passphrase);
    } catch (err) {
      if (!mountedRef.current) return;
      setError(
        err instanceof Error ? err.message : "Failed to generate a password.",
      );
    }
  }, []);

  // Fill the password field on every open and whenever a control changes.
  React.useEffect(() => {
    if (open) void generate(words, separator);
  }, [open, words, separator, generate]);

  return (
    <Popover onOpenChange={setOpen} open={open}>
      {/* Anchor (not Trigger): Radix triggers toggle on click, but repeat
          clicks here must generate a fresh password while the popover stays
          open. Only click-outside or Esc closes it. */}
      <PopoverAnchor asChild>
        <Button
          aria-label="Generate a password"
          className="absolute right-9 top-1/2 h-8 w-8 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          data-testid="backup-passphrase-generate"
          disabled={disabled}
          onClick={() => {
            // The open effect below generates the first password; later
            // clicks re-roll with the current controls.
            if (!open) setOpen(true);
            else void generate(words, separator);
          }}
          ref={anchorRef}
          size="icon"
          type="button"
          variant="ghost"
        >
          <RefreshCw className="h-4 w-4" aria-hidden="true" />
        </Button>
      </PopoverAnchor>
      <PopoverContent
        align="end"
        className="w-72 space-y-3"
        onInteractOutside={(event) => {
          // Clicking the anchor icon is "outside" the content — keep the
          // popover open so that click re-rolls instead of closing.
          if (
            event.target instanceof Node &&
            anchorRef.current?.contains(event.target)
          ) {
            event.preventDefault();
          }
        }}
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <div className="flex items-center justify-between gap-4">
          <label
            className="text-sm text-muted-foreground"
            htmlFor="backup-passphrase-words"
          >
            Words
          </label>
          <div className="flex flex-1 items-center justify-end gap-3">
            <input
              className="h-1.5 w-full max-w-30 cursor-pointer appearance-none rounded-full bg-foreground/15 accent-primary"
              id="backup-passphrase-words"
              data-testid="backup-passphrase-words"
              max={MAX_GENERATED_WORDS}
              min={MIN_GENERATED_WORDS}
              onChange={(event) => setWords(Number(event.target.value))}
              type="range"
              value={words}
            />
            <span className="w-6 text-right text-sm tabular-nums text-foreground">
              {words}
            </span>
          </div>
        </div>

        <div className="flex items-center justify-between gap-4">
          <label
            className="text-sm text-muted-foreground"
            htmlFor="backup-passphrase-separator"
          >
            Separator
          </label>
          <select
            className="h-8 rounded-lg border border-border bg-background px-2 text-sm text-foreground outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            id="backup-passphrase-separator"
            data-testid="backup-passphrase-separator"
            onChange={(event) => setSeparator(event.target.value)}
            value={separator}
          >
            {SEPARATOR_OPTIONS.map((option) => (
              <option key={option.label} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>

        {error ? (
          <p
            className="flex items-start gap-1.5 text-xs text-destructive"
            data-testid="backup-passphrase-generate-error"
          >
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            {error}
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

/**
 * Password-first encrypted key download flow shared by onboarding and
 * Settings. The raw private key never enters this component. Rust creates the
 * NIP-49 payload locally, then the native save dialog produces the user-owned
 * file.
 *
 * The flow is a single password input; a refresh icon inset in the field
 * opens a 1Password-style generator popover (word count + separator).
 * Encryption starts eagerly once the password is valid, so Download usually
 * opens the save dialog instantly; clicking mid-encryption queues the
 * download until the KDF finishes.
 */
export function EncryptedBackupCreator({
  variant = "spotlight",
  createButtonPortal,
  createButtonClassName,
  onCreated,
  onSaved,
}: EncryptedBackupCreatorProps) {
  const [state, dispatch] = React.useReducer(
    encryptedBackupReducer,
    initialEncryptedBackupState,
  );
  const [isRevealed, setIsRevealed] = React.useState(false);
  const [savedPath, setSavedPath] = React.useState<string | null>(null);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [isSaving, setIsSaving] = React.useState(false);
  const mountedRef = React.useRef(true);
  // The committed blob we've already kicked a save off for — guards the
  // commit effect against re-running on unrelated re-renders.
  const savedForRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Eager background encryption: start the KDF as soon as the passphrase is
  // valid (debounced against typing). Results are keyed by passphrase in the
  // reducer, so a completion for an edited passphrase is dropped there.
  const pendingPassphrase = pendingEncryptPassphrase(state);
  const skipDebounce = state.downloadPending;
  React.useEffect(() => {
    if (!pendingPassphrase) return;
    let cancelled = false;
    const start = () => {
      if (cancelled || !mountedRef.current) return;
      dispatch({ type: "encrypt-started", passphrase: pendingPassphrase });
      void createNcryptsecBackup(pendingPassphrase)
        .then((ncryptsec) => {
          if (mountedRef.current)
            dispatch({
              type: "encrypt-succeeded",
              passphrase: pendingPassphrase,
              ncryptsec,
            });
        })
        .catch((err: unknown) => {
          if (mountedRef.current)
            dispatch({
              type: "encrypt-failed",
              passphrase: pendingPassphrase,
              message:
                err instanceof Error
                  ? err.message
                  : "Failed to encrypt your key.",
            });
        });
    };
    const timer = window.setTimeout(
      start,
      skipDebounce ? 0 : ENCRYPT_DEBOUNCE_MS,
    );
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [pendingPassphrase, skipDebounce]);

  // Download commit: fires once per committed blob, whether the commit was
  // instant (encryption already done) or resolved a queued download.
  React.useEffect(() => {
    const ncryptsec = state.ncryptsec;
    if (!ncryptsec || savedForRef.current === ncryptsec) return;
    savedForRef.current = ncryptsec;
    onCreated?.();
    setIsSaving(true);
    setSaveError(null);
    void saveNcryptsecCopy(ncryptsec)
      .then((path) => {
        if (mountedRef.current && path) {
          setSavedPath(path);
          onSaved?.(path);
        }
      })
      .catch((err: unknown) => {
        if (mountedRef.current)
          setSaveError(
            err instanceof Error ? err.message : "Failed to save your key.",
          );
      })
      .finally(() => {
        if (mountedRef.current) setIsSaving(false);
      });
  }, [onCreated, onSaved, state.ncryptsec]);

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
          err instanceof Error ? err.message : "Failed to save your key.",
        );
    } finally {
      if (mountedRef.current) setIsSaving(false);
    }
  }, [isSaving, onSaved, state.ncryptsec]);

  const isSpotlight = variant === "spotlight";
  const issue = passphraseIssue(state.passphrase);

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
            Save a copy…
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
        {saveError ? (
          <p className="text-center text-sm text-destructive">{saveError}</p>
        ) : null}
        <p className="text-center text-xs leading-5 text-muted-foreground">
          Keep this file private. You need both the file and its password to
          restore your identity. Buzz cannot reset the password.
        </p>
      </div>
    );
  }

  return (
    <div
      className={cn("mx-auto w-full max-w-[500px] space-y-3 text-left")}
      data-testid="encrypted-backup-creator"
    >
      <div className="relative">
        <Input
          aria-label="Encryption password"
          autoComplete="new-password"
          className="h-10 bg-background pr-19 font-mono"
          data-testid="backup-passphrase-input"
          disabled={state.downloadPending}
          onChange={(event) =>
            dispatch({ type: "set-passphrase", value: event.target.value })
          }
          placeholder={`Password (min ${MIN_PASSPHRASE_LEN} characters)`}
          type={isRevealed ? "text" : "password"}
          value={state.passphrase}
        />
        <Button
          aria-label={isRevealed ? "Hide password" : "Reveal password"}
          className="absolute right-1 top-1/2 h-8 w-8 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          data-testid="backup-passphrase-reveal-toggle"
          onClick={() => setIsRevealed((revealed) => !revealed)}
          size="icon"
          type="button"
          variant="ghost"
        >
          {isRevealed ? (
            <EyeOff className="h-4 w-4" aria-hidden="true" />
          ) : (
            <Eye className="h-4 w-4" aria-hidden="true" />
          )}
        </Button>
        <PassphraseGeneratorPopover
          disabled={state.downloadPending}
          onGenerated={(value) => {
            dispatch({ type: "set-passphrase", value });
            // A generated password must be visible so the user can save it.
            setIsRevealed(true);
          }}
        />
        {issue ? (
          <p
            className="absolute left-1 top-full mt-1 animate-in text-xs text-muted-foreground fade-in slide-in-from-top-1 duration-200 motion-reduce:animate-none"
            data-testid="backup-passphrase-issue"
          >
            {issue}
          </p>
        ) : null}
      </div>

      {state.createError ? (
        <p
          className="text-center text-sm text-destructive"
          data-testid="encrypted-backup-create-error"
        >
          {state.createError}
        </p>
      ) : null}

      {(() => {
        // Absolute spinner: signals the background encryption without
        // shifting the centered button while it appears and disappears.
        const createButton = (
          <div className="relative">
            {isEncrypting(state) || state.downloadPending ? (
              <Spinner
                aria-label="Encrypting your key"
                className="absolute right-full top-1/2 mr-3 h-4 w-4 -translate-y-1/2 border-2"
                data-testid="encrypted-backup-encrypting"
              />
            ) : null}
            <Button
              className={cn("h-9 rounded-full px-6", createButtonClassName)}
              data-testid="encrypted-backup-create"
              disabled={downloadDisabled(state)}
              onClick={() => dispatch({ type: "download-clicked" })}
              type="button"
            >
              {state.downloadPending ? "Downloading once finished" : "Download"}
            </Button>
          </div>
        );
        // `undefined` = inline (settings); `null` = slot not mounted yet
        // (skip a frame rather than flashing the button inline).
        if (createButtonPortal === undefined)
          return <div className="flex justify-center">{createButton}</div>;
        return createButtonPortal
          ? createPortal(createButton, createButtonPortal)
          : null;
      })()}
    </div>
  );
}
