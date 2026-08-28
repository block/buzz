import { Copy, ExternalLink } from "lucide-react";
import * as React from "react";

import type { SubscribeIntent } from "@/features/meetings/api";
import {
  buildLightningUri,
  formatCountdown,
  isInvoiceExpired,
  secondsUntilExpiry,
} from "@/features/meetings/ui/subscribeState";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import { StyledQrCode } from "@/shared/ui/styled-qr-code";

type InvoicePanelProps = {
  intent: SubscribeIntent;
  /** True while a regenerate (fresh `subscribe`) call is in flight. */
  regenerating: boolean;
  onRegenerate: () => void;
};

/** 1Hz clock, only while mounted — drives the countdown + local expiry flip. */
function useNow(): number {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(id);
  }, []);
  return now;
}

export function InvoicePanel({
  intent,
  regenerating,
  onRegenerate,
}: InvoicePanelProps) {
  const now = useNow();
  const expired = isInvoiceExpired(intent, now);
  const secondsLeft = secondsUntilExpiry(intent.expires_at, now);
  const lightningUri = buildLightningUri(intent.bolt11);

  if (expired) {
    return (
      <div
        className="space-y-3 rounded-xl border border-border/70 bg-muted/40 p-4 text-center"
        data-testid="meeting-invoice-expired"
      >
        <p className="text-sm font-medium">Invoice expired</p>
        <p className="text-xs text-muted-foreground">
          The payment window closed before this invoice was paid.
        </p>
        <Button
          disabled={regenerating}
          onClick={onRegenerate}
          size="sm"
          type="button"
        >
          {regenerating ? "Creating…" : "Get a new invoice"}
        </Button>
      </div>
    );
  }

  return (
    <div
      className="space-y-4 rounded-xl border border-border/70 p-4"
      data-testid="meeting-invoice-panel"
    >
      <div className="flex items-baseline justify-between gap-2">
        <p className="text-sm font-medium">
          Pay {intent.amount_sats.toLocaleString()} sats
        </p>
        <p
          className="text-xs tabular-nums text-muted-foreground"
          data-testid="meeting-invoice-countdown"
        >
          Expires in {formatCountdown(secondsLeft)}
        </p>
      </div>

      <div className="flex justify-center">
        <StyledQrCode
          size={220}
          title="Lightning invoice QR code"
          value={lightningUri}
        />
      </div>

      <p className="break-all rounded-lg bg-muted/60 p-2 font-mono text-xs">
        {intent.bolt11}
      </p>

      <div className="flex flex-wrap gap-2">
        <Button
          onClick={() => copyTextToClipboard(intent.bolt11, "Invoice copied")}
          size="sm"
          type="button"
          variant="outline"
        >
          <Copy />
          Copy invoice
        </Button>
        <Button asChild size="sm" type="button" variant="outline">
          <a href={lightningUri}>
            <ExternalLink />
            Open in wallet
          </a>
        </Button>
      </div>

      <p className="text-xs text-muted-foreground">
        Waiting for payment — this updates automatically once your wallet pays.
      </p>
    </div>
  );
}
