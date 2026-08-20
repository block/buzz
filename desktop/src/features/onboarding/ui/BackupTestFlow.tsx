import { Check, CircleHelp, Eye, EyeOff, FileKey2, FileUp } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { createPortal } from "react-dom";

import {
  getNsec,
  verifyNcryptsecBackup,
  type BackupVerification,
} from "@/shared/api/tauriIdentity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { PubKey } from "@/shared/ui/PubKey";
import { Spinner } from "@/shared/ui/spinner";
import {
  ONBOARDING_SECURITY_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";

type BackupTestStage = "drop" | "password" | "success";

/**
 * Durable progress through the test flow. Owned by the host so navigating
 * away (e.g. onboarding Back) and returning doesn't force the user to
 * re-drop the file. The password attempt is deliberately NOT part of this
 * state — it lives only in short-lived component state and is cleared the
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
  /** "spotlight" is the onboarding treatment; "boxed" fits settings cards. */
  variant?: "spotlight" | "boxed";
  /**
   * When supplied, only this exact just-created file is accepted — the
   * onboarding ceremony proves the user saved *that* backup. Without it the
   * flow is a general-purpose tester for any key backup file.
   */
  expectedNcryptsec?: string;
  /** Re-open the native save dialog for another copy of the backup file. */
  onSaveCopy?: () => void;
  isSaving?: boolean;
  saveError?: string | null;
  /** Optional onboarding footer target for the verification CTA. */
  verifyButtonPortal?: HTMLElement | null;
  /** Host-owned progress so it survives this component unmounting. */
  progress: BackupTestProgress;
  onProgressChange: React.Dispatch<React.SetStateAction<BackupTestProgress>>;
  /** Fired once when the user completes the test successfully. */
  onVerified?: () => void;
};

const BURST_EMOJIS = ["🎉", "✨", "🐝", "🍯", "🔑", "💛"] as const;
const BURST_PARTICLE_COUNT = 18;
const VERIFICATION_CONNECTOR_DOTS = [
  "verification-dot-1",
  "verification-dot-2",
  "verification-dot-3",
  "verification-dot-4",
] as const;
const VERIFICATION_DOT_ANIMATION = {
  opacity: [0.35, 1, 0.35],
  scale: [0.85, 1.25, 0.85],
};
const VERIFICATION_DOT_TRANSITION = {
  duration: 0.7,
  ease: "easeInOut" as const,
  repeat: Number.POSITIVE_INFINITY,
  repeatDelay: 1.2,
};
const PRIVATE_KEY_MASK = Array.from({ length: 63 }, () => "•").join("\u200b");

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

function VerificationConnector({
  delayOffset,
  reduceMotion,
}: {
  delayOffset: number;
  reduceMotion: boolean;
}) {
  return (
    <div
      aria-hidden
      className="my-5 flex h-14 flex-col items-center justify-between py-1"
    >
      {VERIFICATION_CONNECTOR_DOTS.map((dot, index) => (
        <motion.span
          animate={reduceMotion ? undefined : VERIFICATION_DOT_ANIMATION}
          className="block size-1.5 rounded-full bg-foreground/65"
          initial={reduceMotion ? false : { opacity: 0.35, scale: 0.85 }}
          key={dot}
          transition={
            reduceMotion
              ? undefined
              : {
                  ...VERIFICATION_DOT_TRANSITION,
                  delay:
                    (VERIFICATION_CONNECTOR_DOTS.length -
                      1 -
                      index +
                      delayOffset) *
                    0.16,
                }
          }
        />
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
  variant = "spotlight",
  expectedNcryptsec,
  onSaveCopy,
  isSaving = false,
  saveError,
  verifyButtonPortal,
  progress,
  onProgressChange,
  onVerified,
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
  const [successNsec, setSuccessNsec] = React.useState<string | null>(null);
  const [isSuccessNsecRevealed, setIsSuccessNsecRevealed] =
    React.useState(false);
  const [isLoadingSuccessNsec, setIsLoadingSuccessNsec] = React.useState(false);
  const [successNsecError, setSuccessNsecError] = React.useState<string | null>(
    null,
  );
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
        setError(
          expectedNcryptsec
            ? "That doesn't look like your key backup. Choose the file you just downloaded."
            : "That doesn't look like a key backup file.",
        );
        return;
      }
      if (expectedNcryptsec && text !== expectedNcryptsec.trim()) {
        setError("That's a key backup, but not the one you just downloaded.");
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
    [expectedNcryptsec, onProgressChange],
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
      onVerified?.();
    } catch (err) {
      if (mountedRef.current && requestId === requestRef.current)
        setError(
          err instanceof Error ? err.message : "Could not verify this backup.",
        );
    } finally {
      if (mountedRef.current && requestId === requestRef.current)
        setIsVerifying(false);
    }
  }, [attempt, isVerifying, ncryptsec, onProgressChange, onVerified]);

  const toggleSuccessNsec = React.useCallback(async () => {
    if (isSuccessNsecRevealed) {
      setIsSuccessNsecRevealed(false);
      return;
    }
    if (successNsec) {
      setIsSuccessNsecRevealed(true);
      return;
    }
    setIsLoadingSuccessNsec(true);
    setSuccessNsecError(null);
    try {
      const value = await getNsec();
      if (!mountedRef.current) return;
      setSuccessNsec(value);
      setIsSuccessNsecRevealed(true);
    } catch (err) {
      if (!mountedRef.current) return;
      setSuccessNsecError(
        err instanceof Error ? err.message : "Could not retrieve your key.",
      );
    } finally {
      if (mountedRef.current) setIsLoadingSuccessNsec(false);
    }
  }, [isSuccessNsecRevealed, successNsec]);

  const isSpotlight = variant === "spotlight";

  if (stage === "success" && result) {
    // The onboarding ceremony pins the exact file, so a success there is by
    // construction the current identity — celebrate and move on. The general
    // tester reports which identity the backup unlocks.
    const isCeremony = Boolean(expectedNcryptsec);
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
          {isCeremony ? (
            <div className="w-full max-w-140">
              <p className="text-lg font-medium text-foreground">
                Your backup works!
              </p>
              <p className="mt-1.5 text-sm leading-6 text-muted-foreground">
                File and password verified. Keep them both somewhere safe —
                that&apos;s all you need to restore your identity.
              </p>
              <div className="mx-auto mt-4 flex max-w-110 min-w-0 items-center gap-2 text-left">
                <p
                  className={cn(
                    "min-w-0 flex-1 break-all font-mono text-base leading-6 wrap-anywhere",
                    isSuccessNsecRevealed
                      ? "select-text text-foreground"
                      : "select-none blur-[2px] text-muted-foreground",
                  )}
                  data-testid="backup-success-nsec-value"
                >
                  {isSuccessNsecRevealed && successNsec
                    ? successNsec
                    : PRIVATE_KEY_MASK}
                </p>
                <Button
                  aria-label={
                    isSuccessNsecRevealed
                      ? "Hide unlocked private key"
                      : "Reveal unlocked private key"
                  }
                  className="size-9 shrink-0 text-muted-foreground hover:text-foreground"
                  data-testid="backup-success-nsec-toggle"
                  disabled={isLoadingSuccessNsec}
                  onClick={() => void toggleSuccessNsec()}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  {isLoadingSuccessNsec ? (
                    <Spinner className="size-4 border-2" />
                  ) : isSuccessNsecRevealed ? (
                    <EyeOff aria-hidden="true" className="size-4" />
                  ) : (
                    <Eye aria-hidden="true" className="size-4" />
                  )}
                </Button>
              </div>
              {successNsecError ? (
                <p
                  className="mt-2 text-xs text-destructive"
                  data-testid="backup-success-nsec-error"
                  role="alert"
                >
                  {successNsecError}
                </p>
              ) : null}
            </div>
          ) : (
            <>
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
            </>
          )}
        </motion.div>
        {isCeremony ? null : (
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
        )}
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
        <motion.div
          animate={{ opacity: 1, y: 0 }}
          className="relative space-y-4"
          initial={reduceMotion ? false : { opacity: 0, y: 10 }}
          key="drop"
          transition={{ duration: reduceMotion ? 0 : 0.3, ease: "easeOut" }}
        >
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
          <Button
            className={cn(
              "mx-auto",
              isSpotlight
                ? ONBOARDING_SECURITY_PRIMARY_CTA_CLASS
                : "h-9 px-6 text-primary-foreground",
            )}
            data-testid="backup-test-dropzone"
            onClick={() => fileInputRef.current?.click()}
            type="button"
          >
            <span className="font-medium text-sm">Select your backup file</span>
          </Button>
          {isWindowDragging ? (
            /*
             * Composer-style takeover: fills the nearest positioned host
             * surface (the onboarding card / the settings backup row) and is
             * itself the drop target, so anywhere on that surface accepts
             * the file.
             */
            // biome-ignore lint/a11y/noStaticElementInteractions: pointer-only drop target; the select button is the keyboard-accessible path
            <div
              className="absolute inset-2 z-10 mt-0! flex items-center justify-center bg-primary/10 backdrop-blur-sm"
              data-testid="backup-test-drop-overlay"
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => {
                event.preventDefault();
                const file = event.dataTransfer.files?.[0];
                if (file) void handleFile(file);
              }}
            >
              <Card
                className="flex-row items-center gap-2 px-10 py-8 text-sm font-semibold text-foreground"
                textureSize="compact"
                textureTone="dark"
                variant="textured"
              >
                <FileUp aria-hidden="true" className="size-4" />
                <span>Drop your backup file here</span>
              </Card>
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
          {onSaveCopy ? (
            <div className="flex flex-col items-center gap-2">
              <Button
                className={cn(
                  "gap-1.5",
                  isSpotlight
                    ? ONBOARDING_SECONDARY_CTA_CLASS
                    : "h-9 rounded-full bg-foreground/10 px-6 text-sm hover:bg-foreground/15",
                )}
                data-testid="encrypted-backup-save-copy"
                disabled={isSaving}
                onClick={onSaveCopy}
                type="button"
                variant="ghost"
              >
                {isSaving ? <Spinner className="h-4 w-4 border-2" /> : null}
                Re-download backup
              </Button>
            </div>
          ) : null}
          {saveError ? (
            <p className="text-center text-sm text-destructive">{saveError}</p>
          ) : null}
        </motion.div>
      ) : (
        <motion.div
          animate={{ opacity: 1, y: 0 }}
          className="space-y-4"
          initial={reduceMotion ? false : { opacity: 0, y: 10 }}
          key="password"
          transition={{ duration: reduceMotion ? 0 : 0.3, ease: "easeOut" }}
        >
          {(() => {
            const fileRow = (
              <div
                className="flex max-w-full items-center gap-3 rounded-2xl border border-foreground/15 bg-foreground/10 px-4 py-3 text-foreground shadow-sm animate-in fade-in slide-in-from-bottom-1 duration-300 motion-reduce:animate-none"
                data-testid="backup-test-file-accepted"
              >
                <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-foreground/10">
                  <FileKey2
                    aria-hidden="true"
                    className="size-5 text-foreground/80"
                  />
                </span>
                <span className="max-w-70 truncate font-mono text-sm">
                  {fileName}
                </span>
              </div>
            );
            const passwordField = (
              <div className="relative w-full">
                <Input
                  aria-label="Backup password"
                  autoComplete="off"
                  className={cn(
                    "font-mono",
                    isSpotlight
                      ? "h-14 rounded-2xl border-black/20 bg-white px-14 text-center text-lg text-black/80 shadow-none placeholder:text-black/55 focus-visible:ring-black/35"
                      : "h-10 bg-background pr-10",
                  )}
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
                  className={cn(
                    "absolute right-1 top-1/2 h-8 w-8 -translate-y-1/2 text-muted-foreground hover:text-foreground",
                    isSpotlight &&
                      "text-black/55 hover:bg-black/5 hover:text-black/80",
                  )}
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
            );
            if (!isSpotlight) {
              return (
                <>
                  {fileRow}
                  <p className="text-center text-sm leading-6 text-muted-foreground">
                    Enter the password to prove you can unlock this backup.
                  </p>
                  {passwordField}
                </>
              );
            }
            return (
              <div className="flex w-full flex-col items-center">
                <CircleHelp
                  aria-hidden="true"
                  className="size-10 text-foreground/85"
                />
                <VerificationConnector
                  delayOffset={VERIFICATION_CONNECTOR_DOTS.length}
                  reduceMotion={reduceMotion}
                />
                {passwordField}
                <VerificationConnector
                  delayOffset={0}
                  reduceMotion={reduceMotion}
                />
                {fileRow}
              </div>
            );
          })()}
          {(() => {
            const verifyButton = (
              <Button
                className={cn(
                  "font-medium",
                  isSpotlight
                    ? ONBOARDING_SECURITY_PRIMARY_CTA_CLASS
                    : "h-9 px-6 text-sm text-primary-foreground",
                )}
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
            );
            if (verifyButtonPortal === undefined) {
              return (
                <div className="flex justify-center pt-2">{verifyButton}</div>
              );
            }
            return verifyButtonPortal
              ? createPortal(verifyButton, verifyButtonPortal)
              : null;
          })()}
        </motion.div>
      )}
    </div>
  );
}
