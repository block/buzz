import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
  TriangleAlert,
  X,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import {
  cancelPairing,
  confirmPairingSas,
  startPairing,
} from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import { StyledQrCode } from "@/shared/ui/styled-qr-code";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
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

function PairingStatusDialog({
  onClose,
  onConfirm,
  onDeny,
  sasCode,
  step,
}: {
  onClose: () => void;
  onConfirm: () => void;
  onDeny: () => void;
  sasCode: string | null;
  step: PairingStep;
}) {
  const open = step === "sas" || step === "transferring" || step === "done";

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose();
      }}
      open={open}
    >
      <DialogContent
        className="max-w-md gap-0 overflow-hidden border-0 px-6 pb-6 pt-6"
        data-testid="mobile-pairing-dialog"
      >
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 pb-5 pr-8">
            <DialogTitle>Pair mobile device</DialogTitle>
            <DialogDescription>
              {step === "sas"
                ? "Verify the security code matches your mobile device."
                : step === "done"
                  ? "Your mobile device is now paired."
                  : "Securely sending your identity to the mobile app."}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-y-auto pt-4">
            {step === "sas" && sasCode ? (
              <div className="space-y-4">
                <div className="flex flex-col items-center gap-3 py-4">
                  <ShieldCheck className="h-10 w-10 text-primary" />
                  <p className="text-sm font-medium">
                    Verify this code matches your mobile device
                  </p>
                  <div className="rounded-xl border-2 border-primary/30 bg-primary/5 px-8 py-4">
                    <p
                      className="font-mono text-4xl font-bold tracking-[0.3em]"
                      data-testid="pairing-sas-code"
                    >
                      {sasCode.slice(0, 3)} {sasCode.slice(3)}
                    </p>
                  </div>
                  <p className="text-center text-xs text-muted-foreground">
                    You are about to transfer your Buzz identity to another
                    device. Only confirm if you initiated this pairing.
                  </p>
                </div>

                <div className="flex gap-2">
                  <Button
                    className="flex-1"
                    data-testid="deny-sas"
                    onClick={onDeny}
                    variant="outline"
                  >
                    <X className="mr-1.5 h-4 w-4" />
                    Cancel
                  </Button>
                  <Button
                    className="flex-1"
                    data-testid="confirm-sas"
                    onClick={onConfirm}
                  >
                    <Check className="mr-1.5 h-4 w-4" />
                    Codes match
                  </Button>
                </div>
              </div>
            ) : step === "transferring" ? (
              <div className="flex flex-col items-center justify-center gap-3 py-8">
                <LoaderCircle
                  aria-hidden="true"
                  className="h-6 w-6 animate-spin text-muted-foreground"
                />
                <p className="text-sm text-muted-foreground">
                  Sending identity to mobile device...
                </p>
              </div>
            ) : step === "done" ? (
              <div
                className="flex flex-col items-center justify-center gap-3 py-8"
                data-testid="mobile-pairing-done"
              >
                <div className="flex h-12 w-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
                  <Check className="h-6 w-6 text-green-600 dark:text-green-400" />
                </div>
                <p className="text-sm font-medium">Mobile device paired</p>
                <p className="text-center text-xs text-muted-foreground">
                  Your mobile app is now connected to this relay.
                </p>
              </div>
            ) : null}
          </div>
        </div>
      </DialogContent>
    </Dialog>
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

  function handleStatusDialogClose() {
    pairingActiveRef.current = false;
    if (stepRef.current === "done") {
      setStep("idle");
      setQrUri(null);
      setSasCode(null);
      setError(null);
      return;
    }

    cancelPairing().catch(() => {});
    setError("Pairing was canceled.");
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
        className="mx-auto w-fit max-w-full"
        data-testid="mobile-pairing-card"
      >
        <SettingsOptionRow className="flex-col items-stretch justify-start gap-3 p-4">
          <div
            className="flex min-h-[266px] w-[266px] shrink-0 items-center justify-center rounded-lg border border-border/70 bg-white p-3"
            data-testid="mobile-pairing-qr-container"
          >
            {step === "qr" && qrUri ? (
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

          {step === "qr" && qrUri ? (
            <Button
              className="w-full origin-top animate-in fade-in-0 zoom-in-95 duration-[250ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:animate-none"
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
        </SettingsOptionRow>
      </SettingsOptionGroup>

      <PairingStatusDialog
        onClose={handleStatusDialogClose}
        onConfirm={() => void handleConfirmSas()}
        onDeny={handleDenySas}
        sasCode={sasCode}
        step={step}
      />
    </section>
  );
}
