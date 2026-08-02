import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import { Check, LoaderCircle, RefreshCw, ShieldCheck } from "lucide-react";

import { cancelPairing, confirmPairingSas } from "@/shared/api/tauri";
import { startIdentityRecoveryPairing } from "@/shared/api/tauriPairing";
import { Button } from "@/shared/ui/button";
import { StyledQrCode } from "@/shared/ui/styled-qr-code";

type Step = "loading" | "qr" | "sas" | "receiving" | "done" | "error";

export function IdentityRecoveryPairing({
  onRecovered,
}: {
  onRecovered: () => Promise<void>;
}) {
  const [step, setStep] = React.useState<Step>("loading");
  const [qrUri, setQrUri] = React.useState<string | null>(null);
  const [sas, setSas] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const active = React.useRef(true);

  const start = React.useCallback(async () => {
    active.current = true;
    setStep("loading");
    setError(null);
    setSas(null);
    setQrUri(null);
    try {
      setQrUri(await startIdentityRecoveryPairing());
      setStep("qr");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not start recovery.",
      );
      setStep("error");
    }
  }, []);

  React.useEffect(() => {
    void start();
    const unlisteners: Array<() => void> = [];
    let disposed = false;
    listen<{ sas: string }>("pairing-sas-received", ({ payload }) => {
      if (!disposed && active.current) {
        setSas(payload.sas);
        setStep("sas");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    listen("pairing-complete", () => {
      if (!disposed && active.current) {
        active.current = false;
        setStep("done");
        void onRecovered();
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    listen<{ message: string }>("pairing-error", ({ payload }) => {
      if (!disposed && active.current) {
        active.current = false;
        setError(payload.message);
        setStep("error");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    listen<{ reason: string }>("pairing-aborted", ({ payload }) => {
      if (!disposed && active.current) {
        active.current = false;
        setError(`Recovery stopped: ${payload.reason}`);
        setStep("error");
      }
    }).then((unlisten) => (disposed ? unlisten() : unlisteners.push(unlisten)));
    return () => {
      disposed = true;
      active.current = false;
      for (const unlisten of unlisteners) unlisten();
      void cancelPairing();
    };
  }, [onRecovered, start]);

  async function confirm() {
    setStep("receiving");
    try {
      await confirmPairingSas();
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not confirm recovery.",
      );
      setStep("error");
    }
  }

  return (
    <div
      className="mb-8 flex w-full flex-col items-center rounded-3xl border border-foreground/15 bg-background/55 p-6"
      data-testid="identity-recovery-pairing"
    >
      <h2 className="text-xl font-medium">Scan with Buzz on your phone</h2>
      <p className="mt-2 max-w-md text-sm leading-6 text-foreground/70">
        On your signed-in phone, open Settings → Send identity to desktop. This
        code expires shortly and works once.
      </p>
      <div className="mt-5 flex min-h-[252px] min-w-[252px] items-center justify-center rounded-2xl bg-white p-3">
        {step === "qr" && qrUri ? (
          <StyledQrCode
            centerImageSrc="/app-icon@2x.png"
            data-testid="identity-recovery-qr"
            size={228}
            title="Desktop identity recovery QR code"
            value={qrUri}
          />
        ) : step === "sas" && sas ? (
          <div className="flex flex-col items-center gap-4 text-foreground">
            <ShieldCheck className="h-10 w-10" />
            <p
              className="font-mono text-4xl font-bold tracking-[0.25em]"
              data-testid="identity-recovery-sas"
            >
              {sas.slice(0, 3)} {sas.slice(3)}
            </p>
            <Button onClick={() => void confirm()}>
              <Check className="mr-2 h-4 w-4" />
              Codes match
            </Button>
          </div>
        ) : step === "done" ? (
          <div className="flex flex-col items-center gap-3 text-foreground">
            <Check className="h-10 w-10" />
            <p>Identity received securely</p>
          </div>
        ) : step === "error" ? (
          <div className="max-w-52 text-center text-foreground">
            <p className="text-sm text-destructive">{error}</p>
            <Button
              className="mt-4"
              onClick={() => void start()}
              variant="outline"
            >
              <RefreshCw className="mr-2 h-4 w-4" />
              Try again
            </Button>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-3 text-foreground">
            <LoaderCircle className="h-7 w-7 animate-spin" />
            <p className="text-sm">
              {step === "receiving"
                ? "Waiting for your phone to send…"
                : "Creating secure code…"}
            </p>
          </div>
        )}
      </div>
      <p className="mt-4 max-w-md text-xs leading-5 text-foreground/65">
        Your phone will grant this desktop permanent access to your full Buzz
        identity. Only approve a desktop you trust and verify the six-digit code
        on both screens.
      </p>
    </div>
  );
}
