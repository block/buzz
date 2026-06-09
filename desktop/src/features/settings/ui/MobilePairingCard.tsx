import { useCallback, useEffect, useRef, useState } from "react";
import { QRCodeSVG } from "qrcode.react";
import {
  Check,
  Copy,
  ShieldCheck,
  Smartphone,
  TriangleAlert,
  X,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import { Spinner } from "@/shared/ui/spinner";

import {
  cancelPairing,
  confirmPairingSas,
  startPairing,
} from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

type PairingStep =
  | "generating"
  | "qr"
  | "sas"
  | "transferring"
  | "done"
  | "error";

function PairingDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [step, setStep] = useState<PairingStep>("generating");
  const [qrUri, setQrUri] = useState<string | null>(null);
  const [sasCode, setSasCode] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const stepRef = useRef(step);
  stepRef.current = step;

  // Start pairing when dialog opens.
  useEffect(() => {
    if (!open) return;

    setStep("generating");
    setQrUri(null);
    setSasCode(null);
    setError(null);
    let cancelled = false;

    startPairing().then(
      (uri) => {
        if (!cancelled) {
          setQrUri(uri);
          setStep("qr");
        }
      },
      (err) => {
        if (!cancelled) {
          setError(
            err instanceof Error
              ? err.message
              : "Failed to start pairing session",
          );
          setStep("error");
        }
      },
    );

    return () => {
      cancelled = true;
    };
  }, [open]);

  // Listen for Tauri events from the pairing backend.
  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    const unlisteners: (() => void)[] = [];

    listen<{ sas: string }>("pairing-sas-received", (event) => {
      if (!cancelled) {
        setSasCode(event.payload.sas);
        setStep("sas");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    listen("pairing-complete", () => {
      if (!cancelled) {
        setStep("done");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    listen<{ reason: string }>("pairing-aborted", (event) => {
      if (!cancelled) {
        setError(`Pairing aborted: ${event.payload.reason}`);
        setStep("error");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    listen<{ message: string }>("pairing-error", (event) => {
      if (!cancelled) {
        setError(event.payload.message);
        setStep("error");
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    });

    return () => {
      cancelled = true;
      for (const fn of unlisteners) fn();
    };
  }, [open]);

  // Cancel pairing when dialog closes before completion.
  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen && stepRef.current !== "done") {
        cancelPairing().catch(() => {});
      }
      onOpenChange(nextOpen);
    },
    [onOpenChange],
  );

  async function handleConfirmSas() {
    setStep("transferring");
    try {
      await confirmPairingSas();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Failed to send credentials",
      );
      setStep("error");
    }
  }

  function handleDenySas() {
    cancelPairing().catch(() => {});
    setError("SAS code mismatch — pairing cancelled for security.");
    setStep("error");
  }

  async function handleCopy() {
    if (!qrUri) return;
    await navigator.clipboard.writeText(qrUri);
    toast.success("Copied to clipboard");
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent
        className="max-w-md overflow-hidden p-0"
        data-testid="mobile-pairing-dialog"
      >
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Pair Mobile Device</DialogTitle>
            <DialogDescription>
              {step === "sas"
                ? "Verify the security code matches your mobile device."
                : step === "done"
                  ? "Your mobile device is now paired."
                  : "Scan this QR code with the Sprout mobile app to securely pair."}
            </DialogDescription>
          </DialogHeader>

          <div className="flex-1 overflow-y-auto px-6 py-4">
            {step === "error" && error ? (
              <div className="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                <span>{error}</span>
              </div>
            ) : step === "generating" ? (
              <div className="flex flex-col items-center justify-center gap-3 py-8">
                <Spinner className="h-6 w-6 text-muted-foreground" />
                <p className="text-sm text-muted-foreground">
                  Preparing secure pairing session...
                </p>
              </div>
            ) : step === "qr" && qrUri ? (
              <div className="space-y-4">
                <div className="flex justify-center rounded-lg border border-border/70 bg-white p-4">
                  <QRCodeSVG
                    data-testid="mobile-pairing-qr"
                    level="M"
                    size={240}
                    value={qrUri}
                  />
                </div>

                <div className="space-y-1.5">
                  <p className="text-xs font-medium text-muted-foreground">
                    Pairing code
                  </p>
                  <button
                    className="flex w-full min-w-0 items-center gap-2 break-all rounded-lg border border-border bg-muted/50 px-3 py-2 text-left text-xs transition-colors hover:bg-muted/70"
                    data-testid="copy-pairing-code"
                    onClick={handleCopy}
                    title="Copy pairing code"
                    type="button"
                  >
                    <code className="min-w-0 flex-1 break-all">{qrUri}</code>
                    <Copy className="h-3.5 w-3.5 shrink-0" />
                  </button>
                </div>

                <p className="text-center text-xs text-muted-foreground">
                  Waiting for mobile device to scan...
                </p>
              </div>
            ) : step === "sas" && sasCode ? (
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
                    You are about to transfer your Sprout identity to another
                    device. Only confirm if you initiated this pairing.
                  </p>
                </div>

                <div className="flex gap-2">
                  <Button
                    className="flex-1"
                    data-testid="deny-sas"
                    onClick={handleDenySas}
                    variant="outline"
                  >
                    <X className="mr-1.5 h-4 w-4" />
                    Cancel
                  </Button>
                  <Button
                    className="flex-1"
                    data-testid="confirm-sas"
                    onClick={handleConfirmSas}
                  >
                    <Check className="mr-1.5 h-4 w-4" />
                    Codes Match
                  </Button>
                </div>
              </div>
            ) : step === "transferring" ? (
              <div className="flex flex-col items-center justify-center gap-3 py-8">
                <Spinner className="h-6 w-6 text-muted-foreground" />
                <p className="text-sm text-muted-foreground">
                  Sending identity to mobile device...
                </p>
              </div>
            ) : step === "done" ? (
              <div className="flex flex-col items-center justify-center gap-3 py-8">
                <div className="flex h-12 w-12 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/30">
                  <Check className="h-6 w-6 text-green-600 dark:text-green-400" />
                </div>
                <p className="text-sm font-medium">
                  Mobile device paired successfully
                </p>
                <p className="text-center text-xs text-muted-foreground">
                  Your mobile app is now connected to this relay.
                </p>
              </div>
            ) : null}
          </div>

          <div className="flex justify-end border-t border-border/60 bg-background/95 px-6 py-4">
            <Button
              data-testid="mobile-pairing-done"
              onClick={() => handleOpenChange(false)}
              size="sm"
              variant="outline"
            >
              {step === "done" ? "Done" : "Close"}
            </Button>
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
  const [dialogOpen, setDialogOpen] = useState(false);

  return (
    <section className="min-w-0" data-testid="settings-mobile">
      <div className="mb-12 space-y-1">
        <h2 className="text-2xl font-semibold tracking-tight">Mobile</h2>
        <p className="text-base font-normal text-muted-foreground">
          Connect the Sprout mobile app to this relay by scanning a QR code. The
          connection is secured with end-to-end encryption and a verification
          code.
        </p>
      </div>

      <SettingsOptionGroup>
        <SettingsOptionRow className="gap-3">
          <Smartphone className="h-5 w-5 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium">Pair Mobile Device</p>
            <p className="text-sm font-normal text-muted-foreground">
              Securely transfer your identity via NIP-AB protocol
            </p>
          </div>
          <Button
            data-testid="pair-mobile-button"
            disabled={!currentPubkey}
            onClick={() => setDialogOpen(true)}
            size="sm"
          >
            Pair
          </Button>
        </SettingsOptionRow>
      </SettingsOptionGroup>

      {currentPubkey && (
        <PairingDialog onOpenChange={setDialogOpen} open={dialogOpen} />
      )}
    </section>
  );
}
