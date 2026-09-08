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
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import {
  downloadDisabled,
  passphraseIssue,
  pendingEncryptPassphrase,
  encryptedBackupReducer,
  initialEncryptedBackupState,
  MIN_PASSPHRASE_LEN,
  type EncryptedBackupEvent,
  type EncryptedBackupState,
} from "../lib/encryptedBackup";
import {
  type BackupTestProgress,
  BackupTestFlow,
  initialBackupTestProgress,
} from "./BackupTestFlow";
import { BackupPasswordTimeline } from "./BackupPasswordTimeline";
import {
  ONBOARDING_SECURITY_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";

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

const PENDING_TICKER_MESSAGES = [
  "Downloading once finished",
  "Encrypting your password",
  "Just a bit longer...",
] as const;

/** How long each ticker message holds before sliding to the next. */
const PENDING_TICKER_INTERVAL_MS = 2500;

/** Matches the `duration-300` slide transition on the ticker column. */
const PENDING_TICKER_SLIDE_MS = 300;

/**
 * Vertical ticker for the queued-download button label — cycles through the
 * pending messages by sliding a stacked column inside a one-line viewport.
 * The column ends with a clone of the first message, so the wrap-around
 * slides up from the bottom like every other step; once the clone settles,
 * the column snaps (transition disabled) back to the real first row. All
 * lines render at all times, so the button keeps the width of the longest
 * message instead of resizing on each swap.
 */
function PendingDownloadTicker() {
  // Index into the rendered column (messages + trailing clone of the first).
  const [position, setPosition] = React.useState(0);
  const [snap, setSnap] = React.useState(false);

  React.useEffect(() => {
    const timer = window.setInterval(
      () => setPosition((current) => current + 1),
      PENDING_TICKER_INTERVAL_MS,
    );
    return () => window.clearInterval(timer);
  }, []);

  // The clone is visually identical to the first message: once its slide-in
  // finishes, jump back to the real first row without animating.
  React.useEffect(() => {
    if (position !== PENDING_TICKER_MESSAGES.length) return;
    const timer = window.setTimeout(() => {
      setSnap(true);
      setPosition(0);
    }, PENDING_TICKER_SLIDE_MS);
    return () => window.clearTimeout(timer);
  }, [position]);

  // Re-enable the transition one frame after the snap has painted.
  React.useEffect(() => {
    if (!snap) return;
    const raf = window.requestAnimationFrame(() => setSnap(false));
    return () => window.cancelAnimationFrame(raf);
  }, [snap]);

  // The clone row duplicates the first message's text, so it carries its own
  // stable key.
  const column = [
    ...PENDING_TICKER_MESSAGES.map((message) => ({ key: message, message })),
    { key: "wrap-clone", message: PENDING_TICKER_MESSAGES[0] },
  ];

  return (
    <span
      aria-live="polite"
      className="relative block h-5 overflow-hidden"
      data-testid="encrypted-backup-pending-ticker"
    >
      <span
        className={cn(
          "block ease-out",
          snap
            ? "transition-none"
            : "transition-transform duration-300 motion-reduce:transition-none",
        )}
        style={{ transform: `translateY(-${position * 1.25}rem)` }}
      >
        {column.map((row) => (
          <span
            className="flex h-5 items-center justify-center whitespace-nowrap"
            key={row.key}
          >
            {row.message}
          </span>
        ))}
      </span>
    </span>
  );
}

/**
 * Everything about an in-progress backup that must survive this component
 * unmounting: the reducer state (short-lived passphrase + encrypted blob), whether the
 * backup test passed, where the file was saved, the save-once guard, and the
 * test-flow progress. Hosts that need the state to outlive the creator (the
 * onboarding flow, where Back unmounts the step) call
 * `useEncryptedBackupSession` at a longer-lived level and pass it down;
 * otherwise the creator owns a private session internally.
 */
export type EncryptedBackupSession = {
  state: EncryptedBackupState;
  dispatch: React.Dispatch<EncryptedBackupEvent>;
  /**
   * True once the encrypted payload has been committed AND saved to disk.
   * Derived so hosts (e.g. DownloadKeyStep) can branch on it without touching
   * the blob itself — keeping them outside the ncryptsec confinement scan.
   */
  created: boolean;
  /** True once the user has passed the backup test. */
  verified: boolean;
  setVerified: React.Dispatch<React.SetStateAction<boolean>>;
  savedPath: string | null;
  setSavedPath: React.Dispatch<React.SetStateAction<string | null>>;
  /** The committed blob a save was already kicked off for (save-once guard). */
  savedForRef: React.MutableRefObject<string | null>;
  test: BackupTestProgress;
  setTest: React.Dispatch<React.SetStateAction<BackupTestProgress>>;
};

/** Host-side state for `EncryptedBackupCreator` — see `EncryptedBackupSession`. */
export function useEncryptedBackupSession(): EncryptedBackupSession {
  const [state, dispatch] = React.useReducer(
    encryptedBackupReducer,
    initialEncryptedBackupState,
  );
  const [verified, setVerified] = React.useState(false);
  const [savedPath, setSavedPath] = React.useState<string | null>(null);
  const savedForRef = React.useRef<string | null>(null);
  const [test, setTest] = React.useState<BackupTestProgress>(
    initialBackupTestProgress,
  );
  return React.useMemo(
    () => ({
      state,
      dispatch,
      created: state.ncryptsec !== null && savedPath !== null,
      verified,
      setVerified,
      savedPath,
      setSavedPath,
      savedForRef,
      test,
      setTest,
    }),
    [state, verified, savedPath, test],
  );
}

/**
 * Return to a secure saved-password placeholder. The encrypted blob survives
 * for instant re-download, while no password or test attempt is retained.
 */
export function backupSessionToPasswordEntry(
  session: EncryptedBackupSession,
): void {
  session.dispatch({ type: "back-to-password" });
  session.setVerified(false);
  session.setSavedPath(null);
  session.setTest(initialBackupTestProgress);
}

/** Discard all backup-creation and verification progress. */
export function resetEncryptedBackupSession(
  session: EncryptedBackupSession,
): void {
  session.dispatch({ type: "start-new-backup" });
  session.setVerified(false);
  session.setSavedPath(null);
  session.savedForRef.current = null;
  session.setTest(initialBackupTestProgress);
}

type EncryptedBackupCreatorProps = {
  /** "spotlight" is the onboarding treatment; "boxed" fits settings cards. */
  variant?: "spotlight" | "boxed";
  /**
   * When set, the "Download" button is portaled into this element instead of
   * rendering inline.
   */
  createButtonPortal?: HTMLElement | null;
  /** Optional onboarding footer target for the guided-test verification CTA. */
  verifyButtonPortal?: HTMLElement | null;
  /** Extra classes for the "Download" button. */
  createButtonClassName?: string;
  /**
   * Host-owned session so the backup state survives this component
   * unmounting (onboarding Back navigation). Omitted = private session.
   */
  session?: EncryptedBackupSession;
  /** Fired once the encrypted payload has been created (before saving). */
  onCreated?: () => void;
  /** Fired only after the encrypted key file has been saved successfully. */
  onSaved?: (path: string) => void;
  /** Whether creation continues into onboarding's guided test ceremony. */
  guidedTest?: boolean;
  /** Fired once when the user completes the backup test successfully. */
  onVerified?: () => void;
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
  onRequestGenerate,
  onGenerated,
  securityTheme = false,
}: {
  disabled?: boolean;
  onRequestGenerate?: () => void;
  onGenerated: (value: string) => void;
  securityTheme?: boolean;
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
          className={cn(
            "absolute right-9 top-1/2 h-8 w-8 -translate-y-1/2 text-muted-foreground hover:text-foreground",
            securityTheme &&
              "text-black/55 hover:bg-black/5 hover:text-black/80",
          )}
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
        className="w-72 space-y-3 text-foreground"
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
 * opens the save dialog instantly. Background encryption is silent; clicking
 * mid-encryption reveals the queued-download ticker until the KDF finishes.
 */
export function EncryptedBackupCreator({
  variant = "spotlight",
  createButtonPortal,
  verifyButtonPortal,
  createButtonClassName,
  session: sessionProp,
  onCreated,
  onSaved,
  guidedTest = true,
  onVerified,
}: EncryptedBackupCreatorProps) {
  // Hosts without a longer-lived session get a private one (settings card).
  const fallbackSession = useEncryptedBackupSession();
  const session = sessionProp ?? fallbackSession;
  const { state, dispatch, savedPath, setSavedPath, savedForRef } = session;
  const [isRevealed, setIsRevealed] = React.useState(false);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [isSaving, setIsSaving] = React.useState(false);
  const [confirmNewPassword, setConfirmNewPassword] = React.useState(false);
  const mountedRef = React.useRef(true);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // A queued download locks the form — mask the password too so it isn't
  // left readable on screen while the user waits for the save dialog.
  React.useEffect(() => {
    if (state.downloadPending) setIsRevealed(false);
  }, [state.downloadPending]);

  // Correlate KDF completion by an opaque request id. The password exists only
  // in this short-lived effect closure and is cleared from reducer state once
  // Rust returns; stale completions cannot commit.
  const pendingPassphrase = pendingEncryptPassphrase(state);
  const skipDebounce = state.downloadPending;
  React.useEffect(() => {
    if (!pendingPassphrase) return;
    let cancelled = false;
    const requestId = state.nextRequestId;
    const start = () => {
      if (cancelled) return;
      dispatch({ type: "encrypt-started", requestId });
      void createNcryptsecBackup(pendingPassphrase)
        .then((ncryptsec) =>
          dispatch({ type: "encrypt-succeeded", requestId, ncryptsec }),
        )
        .catch((err: unknown) =>
          dispatch({
            type: "encrypt-failed",
            requestId,
            message:
              err instanceof Error
                ? err.message
                : "Failed to encrypt your key.",
          }),
        );
    };
    const timer = window.setTimeout(
      start,
      skipDebounce ? 0 : ENCRYPT_DEBOUNCE_MS,
    );
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [dispatch, pendingPassphrase, skipDebounce, state.nextRequestId]);

  // Download commit: fires once per committed blob, whether the commit was
  // instant (encryption already done) or resolved a queued download. The flow
  // only advances to the test view once the file is actually on disk — a
  // canceled save dialog or a save failure rolls the commit back to the
  // password form so "Download backup" can be clicked again.
  React.useEffect(() => {
    const ncryptsec = state.ncryptsec;
    if (!ncryptsec || savedForRef.current === ncryptsec) return;
    savedForRef.current = ncryptsec;
    onCreated?.();
    setIsSaving(true);
    setSaveError(null);
    const rollBack = () => {
      savedForRef.current = null;
      dispatch({ type: "back-to-password" });
    };
    void saveNcryptsecCopy(ncryptsec)
      .then((path) => {
        if (path) {
          setSavedPath(path);
          onSaved?.(path);
        } else {
          // User canceled the native save dialog — nothing was downloaded.
          rollBack();
        }
      })
      .catch((err: unknown) => {
        rollBack();
        if (mountedRef.current)
          setSaveError(
            err instanceof Error ? err.message : "Failed to save your key.",
          );
      })
      .finally(() => {
        if (mountedRef.current) setIsSaving(false);
      });
  }, [
    dispatch,
    onCreated,
    onSaved,
    savedForRef,
    setSavedPath,
    state.ncryptsec,
  ]);

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
  }, [isSaving, onSaved, setSavedPath, state.ncryptsec]);

  const { setVerified, test, setTest } = session;
  const handleVerified = React.useCallback(() => {
    setVerified(true);
    onVerified?.();
  }, [onVerified, setVerified]);

  const issue = passphraseIssue(state.passphrase);
  const showBackupTimeline =
    variant === "spotlight" &&
    !state.savedPassword &&
    !state.createError &&
    !saveError;

  // The test view requires a successful save, not just a committed blob —
  // while the native save dialog is open the password form stays put.
  if (state.ncryptsec && savedPath && guidedTest) {
    return (
      <div data-testid="encrypted-backup-result">
        <BackupTestFlow
          isSaving={isSaving}
          expectedNcryptsec={state.ncryptsec}
          onProgressChange={setTest}
          onSaveCopy={() => void handleSaveCopy()}
          onVerified={handleVerified}
          progress={test}
          saveError={saveError}
          variant={variant}
          verifyButtonPortal={verifyButtonPortal}
        />
      </div>
    );
  }
  // Without the guided test (settings), a completed save keeps the form
  // visible in its saved-password state: masked input, instant re-download,
  // and the change-password confirmation guarding any edit.

  return (
    <div
      className={cn("mx-auto w-full max-w-[500px] space-y-3 text-left")}
      data-testid="encrypted-backup-creator"
    >
      <div
        className={cn(
          "relative",
          showBackupTimeline && "pb-32 pt-28 [@media(max-height:40rem)]:py-0",
        )}
      >
        {showBackupTimeline ? <BackupPasswordTimeline /> : null}
        <div className="relative z-10">
          <Input
            aria-label="Encryption password"
            autoComplete="new-password"
            autoFocus={variant === "spotlight"}
            className={cn(
              "font-mono",
              variant === "spotlight"
                ? "h-14 rounded-2xl border-black/20 bg-white px-20 text-center text-lg text-black/80 shadow-none placeholder:text-black/55 focus-visible:ring-black/35"
                : "h-10 bg-background pr-19",
            )}
            data-testid="backup-passphrase-input"
            disabled={state.downloadPending}
            readOnly={state.savedPassword}
            aria-describedby={
              state.savedPassword
                ? "backup-saved-password-description"
                : undefined
            }
            onBeforeInput={(event) => {
              if (state.savedPassword) {
                event.preventDefault();
                setConfirmNewPassword(true);
              }
            }}
            onPaste={(event) => {
              if (state.savedPassword) {
                event.preventDefault();
                setConfirmNewPassword(true);
              }
            }}
            onChange={(event) =>
              dispatch({ type: "set-passphrase", value: event.target.value })
            }
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.nativeEvent.isComposing)
                return;
              event.preventDefault();
              if (downloadDisabled(state) || isSaving) return;
              if (state.savedPassword && state.ncryptsec) {
                void handleSaveCopy();
                return;
              }
              dispatch({ type: "download-clicked" });
            }}
            placeholder={
              state.savedPassword
                ? ""
                : `Password (min ${MIN_PASSPHRASE_LEN} characters)`
            }
            type={isRevealed ? "text" : "password"}
            value={state.passphrase}
          />
          {state.savedPassword ? (
            <div
              aria-hidden
              className={cn(
                "pointer-events-none absolute inset-y-0 left-3 flex items-center font-mono tracking-widest text-foreground",
                variant === "spotlight" && "text-black/80",
              )}
              data-testid="backup-saved-password-mask"
            >
              ••••••••••••••••••••••••••••••••
            </div>
          ) : null}
          {state.savedPassword ? (
            <span className="sr-only" id="backup-saved-password-description">
              Backup password saved; hidden for security.
            </span>
          ) : null}
          <Button
            aria-label={
              state.savedPassword
                ? "Change saved backup password"
                : isRevealed
                  ? "Hide password"
                  : "Reveal password"
            }
            className={cn(
              "absolute right-1 top-1/2 h-8 w-8 -translate-y-1/2 text-muted-foreground hover:text-foreground",
              variant === "spotlight" &&
                "text-black/55 hover:bg-black/5 hover:text-black/80",
            )}
            data-testid="backup-passphrase-reveal-toggle"
            disabled={state.downloadPending}
            onClick={() =>
              state.savedPassword
                ? setConfirmNewPassword(true)
                : setIsRevealed((revealed) => !revealed)
            }
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
            onRequestGenerate={
              state.savedPassword
                ? () => setConfirmNewPassword(true)
                : undefined
            }
            onGenerated={(value) => {
              dispatch({ type: "set-passphrase", value });
              // A generated password must be visible so the user can save it.
              setIsRevealed(true);
            }}
            securityTheme={variant === "spotlight"}
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
      </div>

      {state.savedPassword && state.ncryptsec && savedPath ? (
        <div
          className="space-y-1 text-center"
          data-testid="encrypted-backup-created"
        >
          <p
            className="text-xs text-muted-foreground"
            data-testid="encrypted-backup-saved-path"
          >
            Backup saved to {savedPath}
          </p>
          <p className="text-xs leading-5 text-muted-foreground">
            Your password isn't kept — download another copy anytime, or start
            over to choose a new password.
          </p>
        </div>
      ) : null}

      {state.createError ? (
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

      {(() => {
        // A queued download gets an explicit progress treatment. Background
        // encryption stays silent until the user asks to download.
        const createButton = (
          <div className="relative">
            {state.downloadPending || isSaving ? (
              <Spinner
                aria-label="Encrypting your key"
                className="absolute right-full top-1/2 mr-3 h-4 w-4 -translate-y-1/2 border-2"
                data-testid="encrypted-backup-encrypting"
              />
            ) : null}
            <Button
              className={cn("h-9 rounded-full px-6", createButtonClassName)}
              data-testid="encrypted-backup-create"
              disabled={downloadDisabled(state) || isSaving}
              onClick={() =>
                state.savedPassword && state.ncryptsec
                  ? void handleSaveCopy()
                  : dispatch({ type: "download-clicked" })
              }
              type="button"
            >
              {state.downloadPending ? (
                <PendingDownloadTicker />
              ) : state.savedPassword ? (
                "Download backup again"
              ) : (
                "Backup key"
              )}
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
      <AlertDialog
        open={confirmNewPassword}
        onOpenChange={setConfirmNewPassword}
      >
        <AlertDialogContent
          className={cn(
            variant === "spotlight" && "buzz-onboarding-security-theme",
          )}
          data-testid="backup-change-password-dialog"
          surface={variant === "spotlight" ? "textured" : "default"}
          textureSize="compact"
          textureTone={variant === "spotlight" ? "dark" : "light"}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>Create a new backup password?</AlertDialogTitle>
            <AlertDialogDescription>
              Starting over lets you pick a new password and download a fresh
              backup file. Backups you saved earlier will still work — just use
              the password you created them with.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              className={
                variant === "spotlight"
                  ? ONBOARDING_SECONDARY_CTA_CLASS
                  : undefined
              }
            >
              Keep current backup
            </AlertDialogCancel>
            <AlertDialogAction
              className={
                variant === "spotlight"
                  ? ONBOARDING_SECURITY_PRIMARY_CTA_CLASS
                  : undefined
              }
              data-testid="backup-start-new-password"
              onClick={() => {
                dispatch({ type: "start-new-backup" });
                setSavedPath(null);
                savedForRef.current = null;
                setTest(initialBackupTestProgress);
                setIsRevealed(false);
              }}
            >
              Start with a new password
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
