import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";

import { getIdentity } from "@/shared/api/tauriIdentity";
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
import {
  completeEmailImportClaim,
  finalizeSlackOidcAttestation,
  publishImportIdentityClaim,
} from "../lib/importIdentityClaims";
import { assertMatchesSlackJoin } from "../lib/slackMigrationOidc";

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
 * - **oidc**: `via === "oidc"`. Slack verified the user server-side but the
 *   attestation is *not* yet published. During a `join-slack` transaction the
 *   callback is matched to its expected relay and service, then the app redeems
 *   the `code` at `/oidc/finalize` with a freshly signed self-claim. Only after
 *   that creates membership and the attestation does onboarding connect to the
 *   target community and publish the member's consent claim.
 *
 * Completing the dedicated Slack OAuth flow is the consent action for OIDC, so
 * it needs no second confirmation in Buzz. The email fallback retains an
 * explicit confirmation because opening a bearer link is a different flow.
 */
export function ImportClaimDialog() {
  const queryClient = useQueryClient();
  const { transaction, update } = useCommunityOnboarding();
  const [payload, setPayload] =
    React.useState<ImportClaimDeepLinkPayload | null>(null);
  const [phase, setPhase] = React.useState<Phase>("confirm");
  const [error, setError] = React.useState<string | null>(null);
  const transactionRef = React.useRef(transaction);
  const oidcFinalizeRef = React.useRef<string | null>(null);
  transactionRef.current = transaction;

  const close = React.useCallback(() => {
    setPayload(null);
    setError(null);
    setPhase("confirm");
  }, []);

  const receiveClaim = React.useCallback(
    (next: ImportClaimDeepLinkPayload) => {
      if (next.via === "oidc") {
        try {
          const pending = transactionRef.current;
          if (pending?.stage !== "slack-auth") {
            throw new Error(
              "Start Slack sign-in from your team's Slack migration link.",
            );
          }
          assertMatchesSlackJoin(
            next,
            pending.relayUrl,
            pending.slackService,
            pending.slackOidcVerifier,
          );
          const service = next.service;
          const code = next.code;
          if (!service || !code) {
            throw new Error("This Slack response is incomplete.");
          }
          if (oidcFinalizeRef.current === pending.id) return;
          oidcFinalizeRef.current = pending.id;
          void finalizeSlackOidcAttestation({
            service,
            code,
            subject: next.subject,
            verifier: pending.slackOidcVerifier,
          })
            .then(() => {
              update(
                {
                  stage: "connecting",
                  slackSubject: next.subject,
                  slackOidcVerifier: undefined,
                  error: undefined,
                },
                pending.id,
              );
              toast.success(
                "Signed in with Slack — setting up your workspace.",
              );
            })
            .catch((error: unknown) => {
              oidcFinalizeRef.current = null;
              const message =
                error instanceof Error ? error.message : String(error);
              toast.error(`Couldn't finish Slack migration: ${message}`);
            });
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          toast.error(`Couldn't finish Slack migration: ${message}`);
        }
        return;
      }

      setError(null);
      setPhase("confirm");
      setPayload(next);
    },
    [update],
  );

  React.useEffect(() => {
    let cancelled = false;
    const unlistenPromise = listenForImportClaimDeepLinks((next) => {
      if (!cancelled) receiveClaim(next);
    });
    return () => {
      cancelled = true;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [receiveClaim]);

  const confirm = React.useCallback(async () => {
    if (!payload) return;
    setPhase("working");
    setError(null);
    try {
      if (!payload.token || !payload.service) {
        throw new Error("This migration link is incomplete.");
      }
      const { pubkey } = await getIdentity();

      // Email channel: redeem the token so the operator publishes the
      // attestation. The token proves inbox control; the pubkey is ours.
      await completeEmailImportClaim(payload.service, payload.token, pubkey);

      // Email fallback claims run inside an already-connected community.
      await publishImportIdentityClaim(payload.subject);
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
