import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  LoaderCircle,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { toast } from "sonner";

import {
  cancelPairing,
  confirmPairingSas,
  startPairing,
} from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { StyledQrCode } from "@/shared/ui/styled-qr-code";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type PairingStep =
  | "idle"
  | "generating"
  | "qr"
  | "expired"
  | "sas"
  | "transferring"
  | "done"
  | "error";

const PAIRING_CODE_DIGIT_POSITIONS = [0, 1, 2, 3, 4, 5] as const;

function pairingErrorMessage(error: unknown) {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "";

  if (message.toLowerCase().includes("timeout waiting for eose")) {
    return "Pairing took too long. Try again.";
  }

  return message || "We couldn't start pairing. Try again.";
}

function isPairingSessionTimeout(message: string) {
  return message.toLowerCase().includes("session timed out");
}

function PairingStepIndicator({
  complete,
  label,
  testId,
}: {
  complete: boolean;
  label: string;
  testId: string;
}) {
  const shouldReduceMotion = useReducedMotion() ?? false;
  const hiddenState = shouldReduceMotion
    ? { opacity: 0 }
    : { filter: "blur(2px)", opacity: 0, scale: 0.25 };
  const visibleState = shouldReduceMotion
    ? { opacity: 1 }
    : { filter: "blur(0px)", opacity: 1, scale: 1 };

  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative flex h-12 w-12 shrink-0 items-center justify-center rounded-full text-base font-semibold transition-[background-color,color] duration-[250ms] ease-in-out motion-reduce:transition-none",
        complete
          ? "bg-green-600 text-white"
          : "bg-secondary text-secondary-foreground",
      )}
      data-completed={complete ? "true" : "false"}
      data-testid={testId}
    >
      <AnimatePresence initial={false}>
        <motion.span
          animate={visibleState}
          className="absolute inset-0 flex items-center justify-center"
          data-state={complete ? "complete" : "pending"}
          exit={hiddenState}
          initial={hiddenState}
          key={complete ? "complete" : "pending"}
          transition={
            shouldReduceMotion
              ? { duration: 0 }
              : { duration: 0.25, ease: "easeInOut" }
          }
        >
          {complete ? <Check className="h-6 w-6" /> : label}
        </motion.span>
      </AnimatePresence>
    </span>
  );
}

function PairingSteps({ step }: { step: PairingStep }) {
  const hasScanned =
    step === "sas" || step === "transferring" || step === "done";
  const hasConfirmed = step === "transferring" || step === "done";
  const isPaired = step === "done";

  return (
    <ol
      className="flex min-h-[266px] min-w-0 flex-1 flex-col justify-center gap-6 py-2"
      data-testid="mobile-pairing-steps"
    >
      <li className="flex min-w-0 items-start gap-4">
        <PairingStepIndicator
          complete={hasScanned}
          label="1"
          testId="mobile-pairing-scan-step-indicator"
        />
        <div className="min-w-0 pt-0.5">
          <p className="text-base font-medium">Scan QR code</p>
          <p className="mt-1 text-sm text-muted-foreground/80">
            Open Buzz on your mobile device and scan the code shown here.
          </p>
        </div>
      </li>

      <li className="flex min-w-0 items-start gap-4">
        <PairingStepIndicator
          complete={hasConfirmed}
          label="2"
          testId="mobile-pairing-confirm-step-indicator"
        />
        <div className="min-w-0 pt-0.5">
          <p className="text-base font-medium">Confirm mobile code</p>
          <p className="mt-1 text-sm text-muted-foreground/80">
            Check that the six-digit code matches on both devices, then confirm
            it.
          </p>
        </div>
      </li>

      <li
        className="flex min-w-0 items-start gap-4"
        data-testid="mobile-pairing-final-step"
      >
        <PairingStepIndicator
          complete={isPaired}
          label="3"
          testId="mobile-pairing-final-step-indicator"
        />
        <div aria-live="polite" className="min-w-0 pt-0.5">
          <p className="text-base font-medium">
            {isPaired ? "Paired" : "Pair your mobile app"}
          </p>
          <p className="mt-1 text-sm text-muted-foreground/80">
            {isPaired
              ? "Your mobile app is now connected to this relay."
              : "Your mobile app will connect after you confirm the code."}
          </p>
        </div>
      </li>
    </ol>
  );
}

function PairingCodeConfirmation({
  onConfirm,
  onDeny,
  sasCode,
}: {
  onConfirm: () => void;
  onDeny: () => void;
  sasCode: string;
}) {
  const formattedCode = `${sasCode.slice(0, 3)} ${sasCode.slice(3, 6)}`;

  return (
    <div
      className="grid h-[266px] w-full grid-rows-[auto_1fr_auto] text-center"
      data-testid="mobile-pairing-code-confirmation"
    >
      <p
        className="self-start text-base font-medium"
        data-testid="pairing-sas-title"
      >
        Confirm mobile code
      </p>
      <fieldset
        className="flex w-full self-center justify-center gap-[6px]"
        data-testid="pairing-sas-code"
      >
        <legend className="sr-only">Confirmation code {formattedCode}</legend>
        {PAIRING_CODE_DIGIT_POSITIONS.map((position) => (
          <span
            aria-hidden="true"
            className={cn(
              // The cell box is px-frozen on purpose. The digits stay
              // rem-based (`text-2xl`) so they scale with Cmd +/- zoom, but a
              // rem-sized cell (`w-10`) plus rem gaps grew the six cells past
              // this fixed 266px column at 150% text scale and overlapped the
              // step guidance. Freezing the box keeps the code block the same
              // width at every zoom level; a zoomed digit still fits inside.
              "flex w-[40px] shrink-0 items-center justify-center rounded-xl border border-input/60 bg-background py-3 font-mono text-2xl font-semibold text-foreground",
              position === 3 && "ml-[8px]",
            )}
            data-testid={`pairing-sas-code-digit-${position + 1}`}
            key={position}
          >
            {sasCode[position] ?? ""}
          </span>
        ))}
      </fieldset>
      <div
        className="flex w-[240px] flex-col gap-2 self-end justify-self-center"
        data-testid="pairing-sas-actions"
      >
        <Button
          className="w-full"
          data-testid="confirm-sas"
          onClick={onConfirm}
        >
          <Check />
          Codes match
        </Button>
        <Button
          className="w-full"
          data-testid="deny-sas"
          onClick={onDeny}
          variant="outline"
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

export function MobilePairingCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const [step, setStep] = useState<PairingStep>("idle");
  const [qrUri, setQrUri] = useState<string | null>(null);
  const [sasCode, setSasCode] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const requestIdRef = useRef(0);
  const pairingActiveRef = useRef(false);
  const stepRef = useRef(step);
  stepRef.current = step;

  const beginPairing = useCallback(() => {
    const requestId = ++requestIdRef.current;
    pairingActiveRef.current = true;
    setStep("generating");
    setQrUri(null);
    setSasCode(null);
    setError(null);

    startPairing().then(
      (uri) => {
        if (requestId === requestIdRef.current) {
          setQrUri(uri);
          setStep("qr");
        }
      },
      (err) => {
        if (requestId === requestIdRef.current) {
          pairingActiveRef.current = false;
          setError(pairingErrorMessage(err));
          setStep("error");
        }
      },
    );
  }, []);

  useEffect(() => {
    ++requestIdRef.current;
    pairingActiveRef.current = false;
    setStep("idle");
    setQrUri(null);
    setSasCode(null);
    setError(null);

    if (!currentPubkey) {
      return;
    }

    let cancelled = false;
    const unlisteners: (() => void)[] = [];

    listen<{ sas: string }>("pairing-sas-received", (event) => {
      if (!cancelled && pairingActiveRef.current) {
        setSasCode(event.payload.sas);
        setStep("sas");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    listen("pairing-complete", () => {
      if (!cancelled && pairingActiveRef.current) {
        pairingActiveRef.current = false;
        setStep("done");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    listen<{ reason: string }>("pairing-aborted", (event) => {
      if (!cancelled && pairingActiveRef.current) {
        pairingActiveRef.current = false;
        setError(`Pairing stopped: ${event.payload.reason}`);
        setStep("error");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    listen<{ message: string }>("pairing-error", (event) => {
      if (!cancelled && pairingActiveRef.current) {
        pairingActiveRef.current = false;
        if (isPairingSessionTimeout(event.payload.message)) {
          setQrUri(null);
          setSasCode(null);
          setError(null);
          setStep("expired");
          return;
        }

        setError(pairingErrorMessage(event.payload.message));
        setStep("error");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    return () => {
      cancelled = true;
      ++requestIdRef.current;
      pairingActiveRef.current = false;
      for (const fn of unlisteners) fn();
      if (stepRef.current !== "idle" && stepRef.current !== "done") {
        cancelPairing().catch(() => {});
      }
    };
  }, [currentPubkey]);

  async function handleCopy() {
    if (!qrUri) return;
    await writeTextToClipboard(qrUri);
    toast.success("Copied to clipboard");
  }

  async function handleConfirmSas() {
    setStep("transferring");
    try {
      await confirmPairingSas();
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : "We couldn't send your identity. Try again.",
      );
      pairingActiveRef.current = false;
      setStep("error");
    }
  }

  function handleDenySas() {
    pairingActiveRef.current = false;
    cancelPairing().catch(() => {});
    setError("The codes didn't match. Pairing was canceled.");
    setStep("error");
  }

  return (
    <section className="min-w-0" data-testid="settings-mobile">
      <SettingsSectionHeader
        title="Mobile"
        description={
          <>
            Connect the Buzz mobile app to this relay by scanning a QR code. The
            connection is secured with end-to-end encryption and a verification
            code.
          </>
        }
      />

      <SettingsOptionGroup
        className="w-full [container-type:inline-size]"
        data-testid="mobile-pairing-card"
      >
        {/* Persistent polite live region. The pairing steps swap the QR view
            for the inline code confirmation asynchronously, and a screen
            reader would otherwise get no signal that a code is now waiting.
            This stays mounted for every step so the announcement is reliable
            (a region added at the same time as its text often isn't spoken)
            and is visually hidden, so it changes nothing on screen. */}
        <p aria-live="polite" className="sr-only" data-testid="pairing-status">
          {step === "sas" && sasCode
            ? `Verification code ${sasCode.slice(0, 3)} ${sasCode.slice(3, 6)} ready. Check that it matches on your mobile device, then confirm the codes match.`
            : step === "transferring"
              ? "Codes confirmed. Pairing your mobile device."
              : step === "done"
                ? "Your mobile app is now paired."
                : ""}
        </p>
        <SettingsOptionRow
          // Side-by-side is gated on the *card's* own width, not the viewport.
          // `sm:` fires at an 800px viewport, but the settings sidebar and
          // content padding leave the card only ~443px there — the QR column
          // and gap ate all of it, collapsing the step guidance to ~1px and
          // stretching the card past 1000px tall. 46rem is the narrowest card
          // width where the steps column still gets a readable ~294px.
          className="flex-col items-stretch justify-start gap-14 px-6 pb-4 pt-15 [@container(min-width:46rem)]:flex-row [@container(min-width:46rem)]:items-start [@container(min-width:46rem)]:px-15"
          data-testid="mobile-pairing-layout"
        >
          <div className="flex w-[266px] max-w-full shrink-0 flex-col gap-3">
            <div
              className={cn(
                "flex min-h-[266px] w-[266px] max-w-full items-center justify-center rounded-lg border",
                step === "sas" || step === "transferring" || step === "done"
                  ? "border-transparent bg-transparent p-0"
                  : "border-border/70 bg-background p-3",
              )}
              data-testid="mobile-pairing-qr-container"
            >
              {step === "sas" && sasCode ? (
                <PairingCodeConfirmation
                  onConfirm={() => void handleConfirmSas()}
                  onDeny={handleDenySas}
                  sasCode={sasCode}
                />
              ) : step === "transferring" ? (
                <div className="flex flex-col items-center justify-center gap-3 text-center">
                  <LoaderCircle
                    aria-hidden="true"
                    className="h-6 w-6 animate-spin text-muted-foreground"
                    data-testid="pairing-transfer-spinner"
                  />
                  <p className="text-sm text-muted-foreground">
                    Pairing mobile device...
                  </p>
                </div>
              ) : step === "qr" && qrUri ? (
                <StyledQrCode
                  animate
                  centerImageSrc="/app-icon@2x.png"
                  data-testid="mobile-pairing-qr"
                  size={240}
                  title="Mobile pairing QR code"
                  value={qrUri}
                />
              ) : step === "expired" ? (
                <div className="flex max-w-52 origin-center animate-in flex-col items-center gap-3 text-center fade-in-0 zoom-in-95 duration-[250ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:animate-none">
                  <p className="text-sm text-muted-foreground">
                    Pairing code expired.
                  </p>
                  <Button
                    data-testid="regenerate-pairing-button"
                    onClick={beginPairing}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    <RefreshCw className="mr-1.5 h-4 w-4" />
                    Generate new pairing code
                  </Button>
                </div>
              ) : step === "error" ? (
                <div className="flex max-w-52 flex-col items-center gap-3 text-center">
                  <TriangleAlert className="h-6 w-6 text-destructive" />
                  <p className="text-sm text-destructive">
                    {error ?? "Pairing session ended."}
                  </p>
                  <Button
                    data-testid="retry-pairing-button"
                    onClick={beginPairing}
                    size="sm"
                    variant="outline"
                  >
                    Try again
                  </Button>
                </div>
              ) : step === "idle" ? (
                currentPubkey ? (
                  <Button
                    data-testid="start-pairing-button"
                    onClick={beginPairing}
                    type="button"
                  >
                    Start pairing
                  </Button>
                ) : (
                  <p className="max-w-44 text-center text-sm text-muted-foreground">
                    Sign in to generate a mobile pairing code.
                  </p>
                )
              ) : step === "done" ? (
                <div className="flex flex-col items-center gap-3 text-center">
                  <div className="flex h-12 w-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
                    <Check className="h-6 w-6 text-green-600 dark:text-green-400" />
                  </div>
                  <p className="text-base font-medium">Paired</p>
                </div>
              ) : (
                <div className="flex flex-col items-center justify-center gap-3">
                  <LoaderCircle
                    aria-hidden="true"
                    className="h-6 w-6 animate-spin text-muted-foreground"
                    data-testid="pairing-loading-spinner"
                  />
                  <p className="text-sm text-muted-foreground">
                    Starting pairing...
                  </p>
                </div>
              )}
            </div>

            <div className="h-8" data-testid="mobile-pairing-copy-slot">
              {step === "qr" && qrUri ? (
                <Button
                  className="h-8 w-full origin-top animate-in fade-in-0 zoom-in-95 duration-[250ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:animate-none"
                  data-testid="copy-pairing-code"
                  onClick={handleCopy}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <Copy className="mr-1.5 h-4 w-4" />
                  Copy pairing code
                </Button>
              ) : null}
            </div>
          </div>

          <PairingSteps step={step} />
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
