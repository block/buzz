import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Check,
  Copy,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";

import { cancelPairing, confirmPairingSas } from "@/shared/api/tauri";
import { startIdentityRecoveryPairing } from "@/shared/api/tauriPairing";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
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
  const [copied, setCopied] = React.useState(false);
  const active = React.useRef(true);
  const copyTimer = React.useRef<number | null>(null);

  const start = React.useCallback(async () => {
    active.current = true;
    setStep("loading");
    setError(null);
    setSas(null);
    setQrUri(null);
    setCopied(false);
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
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
      void cancelPairing();
    };
  }, [onRecovered, start]);

  async function copyPairingCode() {
    if (!qrUri) return;
    try {
      await writeTextToClipboard(qrUri);
      setCopied(true);
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 2_000);
    } catch {
      setError("Could not copy the pairing code. Try again.");
    }
  }

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
      className="mb-4 flex w-full max-w-[520px] flex-col items-center rounded-3xl border border-foreground/15 bg-background/55 p-4"
      data-testid="identity-recovery-pairing"
    >
      <div className="flex min-h-[220px] min-w-[220px] items-center justify-center rounded-2xl bg-white p-3">
        {step === "qr" && qrUri ? (
          <StyledQrCode
            centerImageSrc="/app-icon@2x.png"
            data-testid="identity-recovery-qr"
            size={196}
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
      {step === "qr" && qrUri ? (
        <Button
          className="mt-3"
          data-testid="copy-identity-recovery-code"
          onClick={() => void copyPairingCode()}
          size="sm"
          type="button"
          variant="outline"
        >
          {copied ? (
            <Check className="mr-2 h-4 w-4" />
          ) : (
            <Copy className="mr-2 h-4 w-4" />
          )}
          {copied ? "Copied" : "Copy pairing code"}
        </Button>
      ) : null}
      {step === "qr" && error ? (
        <p className="mt-2 text-xs text-destructive">{error}</p>
      ) : null}
      <p className="mt-3 max-w-md text-sm leading-5 text-foreground/75">
        On your phone, open Settings → Send identity to desktop. This code
        expires shortly and works once.
      </p>
      <p className="mt-1 max-w-md text-xs leading-4 text-foreground/65">
        Your phone will grant this desktop permanent access to your full Buzz
        identity. Only approve a desktop you trust and verify the six-digit code
        on both screens.
      </p>
    </div>
  );
}
