import * as React from "react";
import { toast } from "sonner";

import { getIdentity } from "@/shared/api/tauriIdentity";
import type { Identity } from "@/shared/api/types";
import type { NostrBindDeepLinkPayload } from "@/shared/deep-link";
import { listenForNostrBindDeepLinks } from "@/shared/deep-link";
import { signNostrIdentityBinding } from "@/features/profile/lib/nostrIdentityBinding";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";

const COPY_SUCCESS_MESSAGE =
  "Signed response copied. Paste it back into the requesting app.";
const EXPIRED_LINK_MESSAGE =
  "This binding link has expired. Request a new one from the requesting app.";

function formatExpiry(expiresAt: string): string {
  const date = new Date(expiresAt);
  if (Number.isNaN(date.getTime())) {
    return expiresAt;
  }
  return date.toLocaleString();
}

function formatError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (error) {
    console.warn("copy signed nostr binding response failed:", error);
    return false;
  }
}

export function NostrBindConsentDialog() {
  const [payload, setPayload] = React.useState<NostrBindDeepLinkPayload | null>(
    null,
  );
  const [identity, setIdentity] = React.useState<Identity | null>(null);
  const [isSigning, setIsSigning] = React.useState(false);
  const [signedResponse, setSignedResponse] = React.useState<string | null>(
    null,
  );
  const [copyFailed, setCopyFailed] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    const unlistenPromise = listenForNostrBindDeepLinks((nextPayload) => {
      setPayload(nextPayload);
      setIdentity(null);
      setSignedResponse(null);
      setCopyFailed(false);
      setError(null);
      getIdentity()
        .then(setIdentity)
        .catch((error) => {
          console.warn("get_identity for nostr bind failed:", error);
          setIdentity(null);
          setError("Could not load the current Buzz identity.");
        });
    });

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const isExpired = React.useMemo(() => {
    if (!payload) {
      return false;
    }
    const expiry = new Date(payload.expiresAt).getTime();
    return Number.isNaN(expiry) || expiry <= Date.now();
  }, [payload]);

  const resetDialog = React.useCallback(() => {
    setPayload(null);
    setSignedResponse(null);
    setCopyFailed(false);
    setError(null);
    setIdentity(null);
    setIsSigning(false);
  }, []);

  const handleOpenChange = React.useCallback(
    (open: boolean) => {
      if (!open) {
        resetDialog();
      }
    },
    [resetDialog],
  );

  const handleSign = React.useCallback(async () => {
    if (!payload) {
      return;
    }
    if (isExpired) {
      setError(EXPIRED_LINK_MESSAGE);
      return;
    }

    setIsSigning(true);
    setError(null);
    setCopyFailed(false);
    try {
      const signed = await signNostrIdentityBinding({
        challengeId: payload.challengeId,
        nonce: payload.nonce,
        verificationCode: payload.verificationCode,
        origin: payload.origin,
        expiresAt: payload.expiresAt,
      });
      setSignedResponse(signed);
      const copied = await copyToClipboard(signed);
      setCopyFailed(!copied);
      if (copied) {
        toast.success(COPY_SUCCESS_MESSAGE);
      } else {
        toast.warning("Signed response ready. Copy it manually below.");
      }
    } catch (error) {
      setError(formatError(error) || "Failed to sign binding response.");
    } finally {
      setIsSigning(false);
    }
  }, [isExpired, payload]);

  const handleCopyAgain = React.useCallback(async () => {
    if (!signedResponse) {
      return;
    }
    const copied = await copyToClipboard(signedResponse);
    setCopyFailed(!copied);
    if (copied) {
      toast.success(COPY_SUCCESS_MESSAGE);
    }
  }, [signedResponse]);

  return (
    <Dialog onOpenChange={handleOpenChange} open={payload !== null}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Bind Buzz identity?</DialogTitle>
          <DialogDescription>
            Buzz will sign a one-time proof. Your private key is not shared.
          </DialogDescription>
        </DialogHeader>

        {payload ? (
          <div className="space-y-4 text-sm">
            <div className="rounded-lg border border-border/60 bg-muted/25 p-4 text-center">
              <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Verification code
              </p>
              <p className="mt-2 font-mono text-4xl font-semibold tracking-[0.35em] text-foreground">
                {payload.verificationCode}
              </p>
              <p className="mt-3 text-muted-foreground">
                Only sign if this code matches the code shown by the requesting
                website.
              </p>
            </div>

            <dl className="space-y-2 rounded-lg border border-border/60 bg-muted/25 p-3">
              <div className="flex justify-between gap-3">
                <dt className="text-muted-foreground">Requesting origin</dt>
                <dd className="break-all text-right font-medium">
                  {payload.origin}
                </dd>
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-muted-foreground">Buzz identity</dt>
                <dd className="break-all text-right font-medium">
                  {identity
                    ? `${identity.displayName} (${truncatePubkey(identity.pubkey)})`
                    : "Loading…"}
                </dd>
              </div>
              <div className="flex justify-between gap-3">
                <dt className="text-muted-foreground">Expires</dt>
                <dd className="text-right font-medium">
                  {formatExpiry(payload.expiresAt)}
                </dd>
              </div>
            </dl>

            {isExpired ? (
              <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive">
                {EXPIRED_LINK_MESSAGE}
              </p>
            ) : null}

            {error ? (
              <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-destructive">
                {error}
              </p>
            ) : null}

            {signedResponse ? (
              <div className="space-y-2">
                <p className="font-medium text-foreground">
                  Signed response {copyFailed ? "ready" : "copied"}. Paste it
                  back into the requesting app.
                </p>
                <Textarea
                  className="max-h-48 min-h-32 font-mono text-xs"
                  readOnly
                  value={signedResponse}
                />
              </div>
            ) : null}
          </div>
        ) : null}

        <DialogFooter>
          <Button
            disabled={isSigning}
            onClick={() => handleOpenChange(false)}
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          {signedResponse ? (
            <Button
              disabled={isSigning}
              onClick={handleCopyAgain}
              type="button"
            >
              Copy response
            </Button>
          ) : (
            <Button
              disabled={isSigning || isExpired || identity === null}
              onClick={handleSign}
              type="button"
            >
              {isSigning ? "Signing…" : "Sign and copy response"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
