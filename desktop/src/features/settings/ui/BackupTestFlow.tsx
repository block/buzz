import { Check, Eye, EyeOff, FileKey2, FileUp } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import {
  verifyNcryptsecBackup,
  type BackupVerification,
} from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { PubKey } from "@/shared/ui/PubKey";
import { Spinner } from "@/shared/ui/spinner";

type BackupTestStage = "drop" | "password" | "success";

/**
 * Progress through the Settings backup-test flow. The password attempt is
 * deliberately NOT part of this state — it lives only in short-lived
 * moment it's submitted or the component unmounts.
 */
export type BackupTestProgress = {
  stage: BackupTestStage;
  /** Name of the accepted file once the drop check passed. */
  fileName: string | null;
  /** Contents of the accepted file, pending or past verification. */
  ncryptsec: string | null;
  /** The Rust-verified public identity once decryption succeeded. */
  result: BackupVerification | null;
};

export const initialBackupTestProgress: BackupTestProgress = {
  stage: "drop",
  fileName: null,
  ncryptsec: null,
  result: null,
};

type BackupTestFlowProps = {
  progress: BackupTestProgress;
  onProgressChange: React.Dispatch<React.SetStateAction<BackupTestProgress>>;
};

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
 * "Test your backup" flow: the user drops a backup file onto a large
 * dropzone, then enters its password. Verification is a real NIP-49 decrypt
 * in Rust — the submitted password is cleared immediately after the result
 * and only the derived public identity ever comes back.
 */
export function BackupTestFlow({
  progress,
  onProgressChange,
}: BackupTestFlowProps) {
  const reduceMotion = useReducedMotion() ?? false;
  const { stage, fileName, ncryptsec, result } = progress;
  // True while a file drag is anywhere over the window — the drop overlay
  // takes over the host surface only for the duration of the drag.
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

  // The password attempt is component-local, never host state: it is cleared
  // when verification is submitted and when this component unmounts.
  const [attempt, setAttempt] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [isVerifying, setIsVerifying] = React.useState(false);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const passwordInputRef = React.useRef<HTMLInputElement | null>(null);
  const mountedRef = React.useRef(true);
  // Opaque correlation id so a stale in-flight verification can't commit
  // after "Use a different file" or unmount.
  const requestRef = React.useRef(0);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      requestRef.current += 1;
      setAttempt("");
    };
  }, []);

  React.useEffect(() => {
    if (stage === "password") passwordInputRef.current?.focus();
  }, [stage]);

  const handleFile = React.useCallback(
    async (file: File) => {
      let text: string;
      try {
        text = (await file.text()).trim();
      } catch {
        if (mountedRef.current) setError("Could not read that file.");
        return;
      }
      if (!mountedRef.current) return;
      if (!text.toLowerCase().startsWith("ncryptsec1")) {
        setError("That doesn't look like a key backup file.");
        return;
      }
      setError(null);
      setAttempt("");
      onProgressChange({
        stage: "password",
        fileName: file.name,
        ncryptsec: text,
        result: null,
      });
    },
    [onProgressChange],
  );

  const handleVerify = React.useCallback(async () => {
    if (!ncryptsec || !attempt || isVerifying) return;
    const password = attempt;
    const requestId = ++requestRef.current;
    setIsVerifying(true);
    setError(null);
    setIsRevealed(false);
    // Clear the attempt the moment it's handed to Rust — success or failure,
    // the typed password never lingers in the field.
    setAttempt("");
    try {
      const verified = await verifyNcryptsecBackup(ncryptsec, password);
      if (!mountedRef.current || requestId !== requestRef.current) return;
      onProgressChange((prev) => ({
        ...prev,
        stage: "success",
        result: verified,
      }));
    } catch (err) {
      if (mountedRef.current && requestId === requestRef.current)
        setError(
          err instanceof Error ? err.message : "Could not verify this backup.",
        );
    } finally {
      if (mountedRef.current && requestId === requestRef.current)
        setIsVerifying(false);
    }
  }, [attempt, isVerifying, ncryptsec, onProgressChange]);

  if (stage === "success" && result) {
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
            This backup works
          </p>
          <p className="mt-1.5 text-sm leading-6 text-muted-foreground">
            {result.matchesCurrentIdentity
              ? "It restores your current Buzz identity."
              : "It restores a different identity than the one signed in here."}
          </p>
          <div className="mt-3 flex justify-center">
            <PubKey
              pubkey={result.pubkey}
              testId="backup-test-npub"
              variant="full"
            />
          </div>
        </motion.div>
        <Button
          className="h-8 rounded-full px-4 text-xs text-muted-foreground hover:text-foreground"
          data-testid="backup-test-another"
          onClick={() => {
            setError(null);
            onProgressChange(initialBackupTestProgress);
          }}
          type="button"
          variant="ghost"
        >
          Test another backup
        </Button>
      </div>
    );
  }

  return (
    <div
      className="mx-auto w-full max-w-125 space-y-4"
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
            className="mx-auto flex h-9 items-center justify-center rounded-full bg-primary px-6 text-center text-primary-foreground shadow transition-colors hover:bg-primary/90"
            data-testid="backup-test-dropzone"
            onClick={() => fileInputRef.current?.click()}
            type="button"
          >
            <span className="text-sm font-medium">Select your backup file</span>
          </button>
          {isWindowDragging ? (
            /*
             * Composer-style takeover: fills the nearest positioned host
             * surface (the settings backup row) and is
             * itself the drop target, so anywhere on that surface accepts
             * the file.
             */
            // biome-ignore lint/a11y/noStaticElementInteractions: pointer-only drop target; the select button is the keyboard-accessible path
            <div
              className="absolute inset-2 z-10 mt-0! flex items-center justify-center rounded-2xl border-2 border-dashed border-primary bg-primary/10 backdrop-blur-sm"
              data-testid="backup-test-drop-overlay"
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => {
                event.preventDefault();
                const file = event.dataTransfer.files?.[0];
                if (file) void handleFile(file);
              }}
            >
              <span className="flex items-center gap-2 rounded-full bg-foreground px-4 py-2 text-sm font-semibold text-background shadow-sm ring-1 ring-background/15">
                <FileUp aria-hidden="true" className="size-4" />
                <span>Drop your backup file here</span>
              </span>
            </div>
          ) : null}
          {error ? (
            <p
              className="text-center text-sm text-destructive"
              data-testid="backup-test-error"
              role="alert"
            >
              {error}
            </p>
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
            That's the one. Now enter your password to prove you can unlock it.
          </p>
          <div className="relative">
            <Input
              aria-label="Backup password"
              autoComplete="off"
              className="h-10 bg-background pr-10 font-mono"
              data-testid="backup-test-password"
              disabled={isVerifying}
              onChange={(event) => setAttempt(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void handleVerify();
                }
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
              disabled={isVerifying}
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
            {error ? (
              <p
                className="absolute left-1 top-full mt-1 text-xs text-destructive animate-in fade-in duration-200 motion-reduce:animate-none"
                data-testid="backup-test-error"
                role="alert"
              >
                {error}
              </p>
            ) : null}
          </div>
          <div className="flex items-center justify-center gap-3 pt-2">
            <Button
              className="h-9 rounded-full bg-primary px-6 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90"
              data-testid="backup-test-verify"
              disabled={!attempt || isVerifying}
              onClick={() => void handleVerify()}
              type="button"
            >
              {isVerifying ? (
                <>
                  <Spinner className="h-4 w-4 border-2" />
                  Checking…
                </>
              ) : (
                "Verify backup"
              )}
            </Button>
            <Button
              className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
              data-testid="backup-test-use-different-file"
              disabled={isVerifying}
              onClick={() => {
                requestRef.current += 1;
                setAttempt("");
                setError(null);
                setIsRevealed(false);
                onProgressChange(initialBackupTestProgress);
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
