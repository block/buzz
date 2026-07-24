import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { KIND_IMPORT_IDENTITY_CLAIM } from "@/shared/constants/kinds";
import {
  type ImportClaimDeepLinkPayload,
  listenForImportClaimDeepLinks,
} from "@/shared/deep-link";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

import { importIdentityBindingsQueryKey } from "../useImportIdentityBindings";

type Phase = "confirm" | "working" | "done" | "error";

/**
 * Completes a zero-touch Slack→Buzz identity migration when the operator's
 * claim-service opens `buzz://import-claim`. Two channels arrive here:
 *
 * - **email**: `token` + `service` present. We POST the token together with
 *   *this device's own* pubkey to `<service>/email/complete`, which makes the
 *   operator publish the owner/admin attestation (kind 30623). Then this app
 *   publishes the subject's self-claim (kind 30624). Only both together
 *   attribute the imported history — so a stray link can, at worst, make the
 *   user consent to an identity no attestation vouches for (inert).
 * - **oidc**: `via === "oidc"`. Slack already verified the user server-side and
 *   the attestation is published; the app only publishes the self-claim.
 *
 * Because the self-claim is a consent signature, we always show an explicit
 * confirm step naming the subject before signing anything.
 */
export function ImportClaimDialog() {
  const queryClient = useQueryClient();
  const [payload, setPayload] =
    React.useState<ImportClaimDeepLinkPayload | null>(null);
  const [phase, setPhase] = React.useState<Phase>("confirm");
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    const unlistenPromise = listenForImportClaimDeepLinks((next) => {
      if (cancelled) return;
      setError(null);
      setPhase("confirm");
      setPayload(next);
    });
    return () => {
      cancelled = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const close = React.useCallback(() => {
    setPayload(null);
    setError(null);
    setPhase("confirm");
  }, []);

  const confirm = React.useCallback(async () => {
    if (!payload) return;
    setPhase("working");
    setError(null);
    try {
      const { pubkey } = await getIdentity();

      // Email channel: redeem the token so the operator publishes the
      // attestation. The token proves inbox control; the pubkey is ours.
      if (payload.token && payload.service) {
        await completeEmailClaim(payload.service, payload.token, pubkey);
      }

      // Both channels: publish the subject's self-claim (the consent half).
      const source = payload.subject.split(":", 1)[0] || "slack";
      const event = await signRelayEvent({
        kind: KIND_IMPORT_IDENTITY_CLAIM,
        content: "",
        tags: [
          ["d", payload.subject],
          ["import", source],
        ],
      });
      await relayClient.publishEvent(
        event,
        "Timed out publishing your identity claim.",
        "Failed to publish your identity claim.",
      );

      await queryClient.invalidateQueries({
        queryKey: importIdentityBindingsQueryKey,
      });
      setPhase("done");
      toast.success("Your imported history is now linked to your account.");
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      setPhase("error");
      toast.error(`Couldn't link your history: ${message}`);
    }
  }, [payload, queryClient]);

  const open = payload !== null;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        // Don't let a click-away abandon an in-flight publish.
        if (!next && phase !== "working") close();
      }}
    >
      <DialogContent
        // Block the backdrop/Escape close while publishing.
        onEscapeKeyDown={(e) => {
          if (phase === "working") e.preventDefault();
        }}
        onPointerDownOutside={(e) => {
          if (phase === "working") e.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>Link your imported history</DialogTitle>
          <DialogDescription>
            {phase === "done"
              ? "Done — your imported messages now show under your account."
              : `This links imported history for ${payload?.subject ?? ""} to this device's Buzz account. Only continue if you started this migration.`}
          </DialogDescription>
        </DialogHeader>

        {phase === "error" && error ? (
          <p className="text-sm text-destructive">{error}</p>
        ) : null}

        <DialogFooter>
          {phase === "done" ? (
            <Button onClick={close}>Close</Button>
          ) : (
            <>
              <Button
                variant="ghost"
                onClick={close}
                disabled={phase === "working"}
              >
                Cancel
              </Button>
              <Button
                onClick={() => void confirm()}
                disabled={phase === "working"}
              >
                {phase === "working"
                  ? "Linking…"
                  : phase === "error"
                    ? "Try again"
                    : "Link my history"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * POST the magic-link token + this device's pubkey to the operator's
 * claim-service. On success the operator has published the attestation.
 */
async function completeEmailClaim(
  service: string,
  token: string,
  pubkey: string,
): Promise<void> {
  const base = service.replace(/\/+$/, "");
  const response = await fetch(`${base}/email/complete`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ token, pubkey }),
    signal: AbortSignal.timeout(30_000),
  });
  const json = (await response.json().catch(() => ({}))) as {
    error?: unknown;
  };
  if (!response.ok) {
    const message =
      typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
}
