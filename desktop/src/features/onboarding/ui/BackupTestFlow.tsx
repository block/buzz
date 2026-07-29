import { Check, Eye, EyeOff, FileKey2, FileUp } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

type BackupTestStage = "drop" | "password" | "success";

/**
 * Durable progress through the test flow. Owned by the host so navigating
 * away (e.g. onboarding Back) and returning doesn't force the user to
 * re-drop the file or retype their attempt.
 */
export type BackupTestProgress = {
  stage: BackupTestStage;
  /** Name of the accepted file once the drop check passed. */
  fileName: string | null;
  /** The password attempt typed so far. */
  attempt: string;
};

export const initialBackupTestProgress: BackupTestProgress = {
  stage: "drop",
  fileName: null,
  attempt: "",
};

type BackupTestFlowProps = {
  /** "spotlight" is the onboarding treatment; "boxed" fits settings cards. */
  variant?: "spotlight" | "boxed";
  /** The exact committed `ncryptsec1…` blob the downloaded file contains. */
  ncryptsec: string;
  /** The passphrase the backup was encrypted under. */
  passphrase: string;
  /** Re-open the native save dialog for another copy of the backup file. */
  onSaveCopy: () => void;
  isSaving: boolean;
  savedPath: string | null;
  saveError: string | null;
  /** Host-owned progress so it survives this component unmounting. */
  progress: BackupTestProgress;
  onProgressChange: React.Dispatch<React.SetStateAction<BackupTestProgress>>;
  /** Fired once when the user completes the test successfully. */
  onVerified?: () => void;
};

/**
 * How long after the last keystroke before a wrong attempt is called out.
 * Verification itself is instant on match; this only delays the scolding.
 */
const MISMATCH_HINT_PAUSE_MS = 900;

const BURST_EMOJIS = ["🎉", "✨", "🐝", "🍯", "🔑", "💛"] as const;
const BURST_PARTICLE_COUNT = 18;

type BurstParticle = {
  id: number;
  x: number;
  y: number;
  emoji: string;
  delay: number;
  scale: number;
  rotate: number;
};

/**
 * One-shot radial emoji burst behind the success badge. Purely decorative —
 * skipped entirely under reduced motion.
 */
function SuccessBurst() {
  const particles = React.useMemo<BurstParticle[]>(
    () =>
      Array.from({ length: BURST_PARTICLE_COUNT }, (_, i) => {
        const angle =
          (i / BURST_PARTICLE_COUNT) * Math.PI * 2 + Math.random() * 0.5;
        const distance = 70 + Math.random() * 80;
        return {
          id: i,
          x: Math.cos(angle) * distance,
          y: Math.sin(angle) * distance,
          emoji: BURST_EMOJIS[i % BURST_EMOJIS.length],
          delay: Math.random() * 0.18,
          scale: 0.8 + Math.random() * 0.7,
          rotate: -120 + Math.random() * 240,
        };
      }),
    [],
  );

  return (
    <div
      aria-hidden
      className="pointer-events-none absolute inset-0 flex items-center justify-center overflow-visible"
    >
      {particles.map((particle) => (
        <motion.span
          animate={{
            x: particle.x,
            y: particle.y,
            opacity: 0,
            scale: particle.scale,
            rotate: particle.rotate,
          }}
          className="absolute text-xl"
          initial={{ x: 0, y: 0, opacity: 1, scale: 0.3, rotate: 0 }}
          key={particle.id}
          transition={{
            duration: 0.9,
            delay: particle.delay,
            ease: "easeOut",
          }}
        >
          {particle.emoji}
        </motion.span>
      ))}
    </div>
  );
}

/**
 * Post-download "Test your backup" flow: the user drops the file they just
 * downloaded onto a large dropzone, then retypes their password. Both checks
 * run instantly in memory — the dropped file's bytes must equal the committed
 * blob and the typed password must equal the passphrase that encrypted it —
 * so there is no second KDF run and no key material beyond what the creator
 * already held.
 */
export function BackupTestFlow({
  variant = "spotlight",
  ncryptsec,
  passphrase,
  onSaveCopy,
  isSaving,
  savedPath,
  saveError,
  progress,
  onProgressChange,
  onVerified,
}: BackupTestFlowProps) {
  const reduceMotion = useReducedMotion() ?? false;
  const { stage, fileName, attempt } = progress;
  const [isDragActive, setIsDragActive] = React.useState(false);
  // True while a file drag is anywhere over the window — the select button
  // renders as a dropzone only for the duration of the drag.
  const [isWindowDragging, setIsWindowDragging] = React.useState(false);
  const dragDepthRef = React.useRef(0);

  React.useEffect(() => {
    // dragenter/dragleave fire per nested element, so track depth to know
    // when the drag has actually left the window.
    const handleDragEnter = (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes("Files")) return;
      dragDepthRef.current += 1;
      setIsWindowDragging(true);
    };
    const handleDragLeave = () => {
      dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
      if (dragDepthRef.current === 0) setIsWindowDragging(false);
    };
    const handleDragEnd = () => {
      dragDepthRef.current = 0;
      setIsWindowDragging(false);
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
  }, []);
  const [fileError, setFileError] = React.useState<string | null>(null);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const passwordInputRef = React.useRef<HTMLInputElement | null>(null);
  const mountedRef = React.useRef(true);
  const verifiedFiredRef = React.useRef(false);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  React.useEffect(() => {
    if (stage === "password") passwordInputRef.current?.focus();
  }, [stage]);

  const succeed = React.useCallback(() => {
    onProgressChange((prev) => ({ ...prev, stage: "success" }));
    if (!verifiedFiredRef.current) {
      verifiedFiredRef.current = true;
      onVerified?.();
    }
  }, [onProgressChange, onVerified]);

  const handleFile = React.useCallback(
    async (file: File) => {
      let text: string;
      try {
        text = await file.text();
      } catch {
        if (mountedRef.current) setFileError("Could not read that file.");
        return;
      }
      if (!mountedRef.current) return;
      const trimmed = text.trim();
      if (trimmed === ncryptsec.trim()) {
        setFileError(null);
        onProgressChange((prev) => ({
          ...prev,
          stage: "password",
          fileName: file.name,
        }));
      } else if (trimmed.toLowerCase().startsWith("ncryptsec1")) {
        setFileError(
          "That's a key backup, but not the one you just downloaded.",
        );
      } else {
        setFileError(
          "That doesn't look like your key backup. Choose the file you just downloaded.",
        );
      }
    },
    [ncryptsec, onProgressChange],
  );

  const handlePasswordChange = React.useCallback(
    (value: string) => {
      onProgressChange((prev) => ({ ...prev, attempt: value }));
      if (value === passphrase) succeed();
    },
    [onProgressChange, passphrase, succeed],
  );

  // Only scold after the user pauses (or presses Enter), never mid-typing —
  // and never based on the attempt's length, which would leak how long the
  // real password is.
  const [mismatchVisible, setMismatchVisible] = React.useState(false);
  React.useEffect(() => {
    setMismatchVisible(false);
    if (stage !== "password" || attempt.length === 0 || attempt === passphrase)
      return;
    const timer = window.setTimeout(
      () => setMismatchVisible(true),
      MISMATCH_HINT_PAUSE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [stage, attempt, passphrase]);

  const isSpotlight = variant === "spotlight";
  const attemptWrong = stage === "password" && mismatchVisible;

  if (stage === "success") {
    return (
      <div
        className="relative flex flex-col items-center gap-4 py-4 text-center"
        data-testid="backup-test-success"
      >
        {reduceMotion ? null : <SuccessBurst />}
        <motion.div
          animate={{ scale: 1, opacity: 1 }}
          className="flex h-16 w-16 items-center justify-center rounded-full bg-primary text-primary-foreground"
          initial={reduceMotion ? false : { scale: 0, opacity: 0 }}
          transition={
            reduceMotion
              ? { duration: 0 }
              : { type: "spring", stiffness: 380, damping: 18 }
          }
        >
          <Check aria-hidden="true" className="h-8 w-8" strokeWidth={3} />
        </motion.div>
        <motion.div
          animate={{ opacity: 1, y: 0 }}
          initial={reduceMotion ? false : { opacity: 0, y: 8 }}
          transition={
            reduceMotion ? { duration: 0 } : { delay: 0.15, duration: 0.35 }
          }
        >
          <p className="text-lg font-medium text-foreground">
            Your backup works!
          </p>
          <p className="mt-1.5 text-sm leading-6 text-muted-foreground">
            File and password verified. Keep them both somewhere safe — that's
            all you need to restore your identity.
          </p>
        </motion.div>
      </div>
    );
  }

  return (
    <div
      className={cn(
        "mx-auto w-full space-y-4",
        isSpotlight ? "max-w-140" : "max-w-125",
      )}
      data-testid="backup-test-flow"
    >
      {stage === "drop" ? (
        <>
          <input
            accept=".ncryptsec,text/plain"
            className="sr-only"
            data-testid="backup-test-file-input"
            onChange={(event) => {
              const file = event.target.files?.[0];
              // Allow re-selecting the same file after an error.
              event.target.value = "";
              if (file) void handleFile(file);
            }}
            ref={fileInputRef}
            tabIndex={-1}
            type="file"
          />
          <button
            className={cn(
              "flex flex-col items-center justify-center gap-2 text-center transition-all",
              isWindowDragging
                ? cn(
                    "w-full rounded-2xl border-2 border-dashed px-6",
                    isSpotlight ? "h-44" : "h-32",
                    isDragActive
                      ? "border-primary bg-primary/10"
                      : "border-foreground/25 bg-background/40",
                  )
                : "mx-auto h-14 rounded-full bg-primary px-12 text-(--buzz-onboarding-cta-label) shadow hover:bg-primary/90",
            )}
            data-testid="backup-test-dropzone"
            onClick={() => fileInputRef.current?.click()}
            onDragLeave={() => setIsDragActive(false)}
            onDragOver={(event) => {
              event.preventDefault();
              setIsDragActive(true);
            }}
            onDrop={(event) => {
              event.preventDefault();
              setIsDragActive(false);
              const file = event.dataTransfer.files?.[0];
              if (file) void handleFile(file);
            }}
            type="button"
          >
            {isWindowDragging ? (
              <>
                <FileUp
                  aria-hidden="true"
                  className={cn(
                    "h-8 w-8 transition-colors",
                    isDragActive ? "text-primary" : "text-muted-foreground",
                  )}
                />
                <span className="text-sm font-medium text-foreground">
                  Drop your backup file here
                </span>
              </>
            ) : (
              <span className="text-base font-medium">
                Select your backup file
              </span>
            )}
          </button>
          {fileError ? (
            <p
              className="text-center text-sm text-destructive"
              data-testid="backup-test-file-error"
            >
              {fileError}
            </p>
          ) : null}
          <div className="flex flex-col items-center gap-2">
            <Button
              className="h-12 gap-1.5 rounded-full bg-foreground/10 px-10 text-base hover:bg-foreground/15"
              data-testid="encrypted-backup-save-copy"
              disabled={isSaving}
              onClick={onSaveCopy}
              type="button"
              variant="ghost"
            >
              {isSaving ? <Spinner className="h-4 w-4 border-2" /> : null}
              Re-download backup
            </Button>
            {savedPath ? (
              <p
                className="text-center text-xs text-muted-foreground"
                data-testid="encrypted-backup-saved-path"
              >
                Saved to {savedPath}
              </p>
            ) : null}
          </div>
          {saveError ? (
            <p className="text-center text-sm text-destructive">{saveError}</p>
          ) : null}
        </>
      ) : (
        <>
          <div
            className="flex items-center justify-center gap-2 text-sm text-foreground animate-in fade-in slide-in-from-bottom-1 duration-300 motion-reduce:animate-none"
            data-testid="backup-test-file-accepted"
          >
            <FileKey2
              aria-hidden="true"
              className="h-4 w-4 text-muted-foreground"
            />
            <span className="max-w-70 truncate font-mono text-xs">
              {fileName}
            </span>
            <Check aria-hidden="true" className="h-4 w-4 text-primary" />
          </div>
          <p className="text-center text-sm leading-6 text-muted-foreground">
            That's the one. Now type your password to prove you can unlock it.
          </p>
          <div className="relative">
            <Input
              aria-label="Backup password"
              autoComplete="off"
              className="h-10 bg-background pr-10 font-mono"
              data-testid="backup-test-password"
              onChange={(event) => handlePasswordChange(event.target.value)}
              onKeyDown={(event) => {
                // Enter is an explicit "check it" — no need to wait out the
                // typing pause before calling out a mismatch.
                if (event.key === "Enter" && attempt.length > 0)
                  setMismatchVisible(attempt !== passphrase);
              }}
              placeholder="Your backup password"
              ref={passwordInputRef}
              type={isRevealed ? "text" : "password"}
              value={attempt}
            />
            <Button
              aria-label={isRevealed ? "Hide password" : "Reveal password"}
              className="absolute right-1 top-1/2 h-8 w-8 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              data-testid="backup-test-password-reveal-toggle"
              onClick={() => setIsRevealed((revealed) => !revealed)}
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
            {attemptWrong ? (
              <p
                className="absolute left-1 top-full mt-1 text-xs text-destructive animate-in fade-in duration-200 motion-reduce:animate-none"
                data-testid="backup-test-password-mismatch"
              >
                Not quite — check the password you saved.
              </p>
            ) : null}
          </div>
          <div className="flex justify-center pt-2">
            <Button
              className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
              data-testid="backup-test-use-different-file"
              onClick={() => {
                onProgressChange(initialBackupTestProgress);
                setIsRevealed(false);
              }}
              size="sm"
              type="button"
              variant="ghost"
            >
              Use a different file
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
