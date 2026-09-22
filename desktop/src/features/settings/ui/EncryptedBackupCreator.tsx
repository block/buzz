import { AlertTriangle, Eye, EyeOff, RefreshCw } from "lucide-react";
import * as React from "react";

import { generateBackupPassphrase } from "@/shared/api/tauriIdentity";
import { useEncryptedBackup } from "@/features/settings/EncryptedBackupProvider";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { downloadDisabled, MIN_PASSPHRASE_LEN } from "../lib/encryptedBackup";

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
 * Indeterminate KDF progress. Scrypt does not expose intermediate progress,
 * so randomized increments consume a shrinking fraction of the remaining
 * distance. The bar moves quickly at first and can never reach completion.
 */
function FakeKdfProgressBar() {
  const [progress, setProgress] = React.useState(0);

  React.useEffect(() => {
    let animationFrame = 0;
    let nextAdvanceAt = 0;
    const advance = (now: number) => {
      if (now >= nextAdvanceAt) {
        setProgress((current) => {
          const remaining = 90 - current;
          const fraction = 0.08 + Math.random() * 0.22;
          return Math.min(90, current + Math.max(0.25, remaining * fraction));
        });
        nextAdvanceAt = now + 180 + Math.random() * 420;
      }
      animationFrame = window.requestAnimationFrame(advance);
    };
    animationFrame = window.requestAnimationFrame(advance);
    return () => window.cancelAnimationFrame(animationFrame);
  }, []);

  return (
    <div
      aria-label="Encrypting your key"
      aria-valuemax={100}
      aria-valuemin={0}
      aria-valuenow={Math.round(progress)}
      className="h-2 w-full overflow-hidden rounded-full bg-muted"
      data-testid="encrypted-backup-progress"
      role="progressbar"
    >
      <div
        className="h-full rounded-full bg-primary transition-[width] duration-300 ease-out motion-reduce:transition-none"
        style={{ width: `${progress}%` }}
      />
    </div>
  );
}

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
  onRequestGenerate,
  onGenerated,
}: {
  disabled?: boolean;
  onRequestGenerate?: () => void;
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
            if (onRequestGenerate) {
              onRequestGenerate();
              return;
            }
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
 * Password-first encrypted key download flow for Settings. The raw private
 * key never enters this component. Rust creates the
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
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const { state, dispatch, isSaving, saveError } = useEncryptedBackup();
  const [isRevealed, setIsRevealed] = React.useState(false);

  // A queued download hides the form; mask the password before it can return
  // in any error state.
  React.useEffect(() => {
    if (state.downloadPending) setIsRevealed(false);
  }, [state.downloadPending]);

  React.useEffect(() => {
    if (state.ncryptsec) onOpenChange(false);
  }, [onOpenChange, state.ncryptsec]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-lg" data-testid="encrypted-backup-dialog">
        <DialogHeader className="pr-8">
          <DialogTitle>Create a key backup</DialogTitle>
          <DialogDescription>
            You can close this window while Buzz finishes the backup in the
            background.
          </DialogDescription>
        </DialogHeader>
        <div
          className="w-full space-y-3 text-left"
          data-testid="encrypted-backup-creator"
        >
          {state.downloadPending ? (
            <FakeKdfProgressBar />
          ) : !state.savedPassword ? (
            <div className="relative">
              <Input
                aria-label="Encryption password"
                autoComplete="new-password"
                className="h-10 bg-background pr-19"
                data-testid="backup-passphrase-input"
                onChange={(event) =>
                  dispatch({
                    type: "set-passphrase",
                    value: event.target.value,
                  })
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
                onGenerated={(value) => {
                  dispatch({ type: "set-passphrase", value });
                  // A generated password must be visible so the user can save it.
                  setIsRevealed(true);
                }}
              />
            </div>
          ) : null}

          {!state.downloadPending && !state.savedPassword ? (
            <p className="text-xs leading-5 text-muted-foreground">
              Keep the file private and save its password somewhere safe — Buzz
              cannot reset it. Once ready, the backup remains available to
              download for 5 minutes.
            </p>
          ) : null}

          {state.createError && state.passphrase.length === 0 ? (
            <p
              className="text-center text-sm text-destructive"
              data-testid="encrypted-backup-create-error"
            >
              {state.createError}
            </p>
          ) : null}

          {saveError ? (
            <p
              className="text-center text-sm text-destructive"
              data-testid="encrypted-backup-save-error"
            >
              {saveError}
            </p>
          ) : null}

          {!state.downloadPending ? (
            <div className="flex justify-end">
              <Button
                className="h-9 rounded-full px-6"
                data-testid="encrypted-backup-create"
                disabled={downloadDisabled(state) || isSaving}
                onClick={() => dispatch({ type: "download-clicked" })}
                type="button"
              >
                Backup key
              </Button>
            </div>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
