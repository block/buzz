import * as React from "react";
import { ShieldCheck } from "lucide-react";

import {
  getIdentityHandoff,
  setIdentityHandoffPolicyReceipt,
} from "@/features/onboarding/identityHandoffVault";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import {
  acceptJoinPolicy,
  getJoinPolicy,
  type JoinPolicy,
} from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { JoinPolicyNotice } from "./JoinPolicyNotice";

const POLICY_DISCOVERY_ERROR =
  "Buzz couldn’t check this community’s requirements. Try again.";
const POLICY_ACCEPTANCE_ERROR =
  "Buzz couldn’t confirm your acceptance. Nothing was claimed; try again.";
const POLICY_CHANGED_ERROR =
  "The community requirements changed. Review and accept the updated version.";

export function IdentityHandoffPolicyStep({
  onCancel,
}: {
  onCancel: () => void;
}) {
  const { transaction, update } = useCommunityOnboarding();
  const [policy, setPolicy] = React.useState<JoinPolicy | null>(null);
  const [ageConfirmed, setAgeConfirmed] = React.useState(false);
  const [agreementConfirmed, setAgreementConfirmed] = React.useState(false);
  const [policyError, setPolicyError] = React.useState<string | null>(null);
  const [isAccepting, setIsAccepting] = React.useState(false);
  const headingRef = React.useRef<HTMLHeadingElement | null>(null);

  const transactionId = transaction?.id;
  const relayUrl = transaction?.relayUrl;
  const stage = transaction?.stage;
  const discoveryError = transaction?.error;

  React.useEffect(() => {
    if (
      !transactionId ||
      !relayUrl ||
      stage !== "policy-checking" ||
      discoveryError
    ) {
      return;
    }

    if (!getIdentityHandoff(transactionId)) {
      update(
        {
          stage: "handoff-terminal",
          handoffTerminalReason: "restart",
          error: undefined,
        },
        transactionId,
      );
      return;
    }

    let cancelled = false;
    setPolicyError(null);
    void getJoinPolicy(relayUrl, "native")
      .then((currentPolicy) => {
        if (cancelled) return;
        if (!currentPolicy) {
          update({ stage: "claiming", error: undefined }, transactionId);
          return;
        }
        setPolicy(currentPolicy);
        setAgeConfirmed(false);
        setAgreementConfirmed(false);
        update({ stage: "policy-consent", error: undefined }, transactionId);
      })
      .catch(() => {
        if (cancelled) return;
        update({ error: POLICY_DISCOVERY_ERROR }, transactionId);
      });

    return () => {
      cancelled = true;
    };
  }, [discoveryError, relayUrl, stage, transactionId, update]);

  React.useEffect(() => {
    if (stage === "policy-consent" && !policy) {
      update({ stage: "policy-checking", error: undefined }, transactionId);
    }
  }, [policy, stage, transactionId, update]);

  React.useLayoutEffect(() => {
    if (
      stage === "policy-checking" ||
      stage === "policy-consent" ||
      discoveryError ||
      policyError
    ) {
      headingRef.current?.focus();
    }
  }, [discoveryError, policyError, stage]);

  const retryDiscovery = () => {
    update({ stage: "policy-checking", error: undefined }, transactionId);
  };

  const acceptCurrentPolicy = async () => {
    if (!transactionId || !relayUrl || !policy || isAccepting) return;
    const credential = getIdentityHandoff(transactionId);
    if (!credential) {
      update(
        {
          stage: "handoff-terminal",
          handoffTerminalReason: "restart",
          error: undefined,
        },
        transactionId,
      );
      return;
    }

    setIsAccepting(true);
    setPolicyError(null);
    try {
      const currentPolicy = await getJoinPolicy(relayUrl, "native");
      if (!currentPolicy) {
        update({ stage: "claiming", error: undefined }, transactionId);
        return;
      }
      if (currentPolicy.version !== policy.version) {
        setPolicy(currentPolicy);
        setAgeConfirmed(false);
        setAgreementConfirmed(false);
        setPolicyError(POLICY_CHANGED_ERROR);
        return;
      }
      if (currentPolicy.ageAttestationRequired && !ageConfirmed) {
        setPolicyError("Confirm that you are at least 18 years old.");
        return;
      }
      if (
        (currentPolicy.termsMarkdown || currentPolicy.privacyMarkdown) &&
        !agreementConfirmed
      ) {
        setPolicyError("Agree to the Terms of Service and Privacy Policy.");
        return;
      }

      const receipt = await acceptJoinPolicy(
        relayUrl,
        credential.code,
        currentPolicy.version,
        ageConfirmed,
      );
      if (!setIdentityHandoffPolicyReceipt(transactionId, receipt)) {
        update(
          {
            stage: "handoff-terminal",
            handoffTerminalReason: "restart",
            error: undefined,
          },
          transactionId,
        );
        return;
      }
      update({ stage: "claiming", error: undefined }, transactionId);
    } catch {
      setPolicyError(POLICY_ACCEPTANCE_ERROR);
    } finally {
      setIsAccepting(false);
    }
  };

  if (stage === "policy-checking") {
    return (
      <div data-testid="identity-handoff-policy-discovery">
        <ShieldCheck aria-hidden="true" className="mx-auto h-10 w-10" />
        <h1
          className="mt-5 text-title font-normal"
          ref={headingRef}
          tabIndex={-1}
        >
          Checking community requirements
        </h1>
        <p aria-live="polite" className="mt-3 text-sm text-foreground/80">
          {discoveryError ?? "Loading the current join policy…"}
        </p>
        <div className="mt-6 flex justify-center gap-3">
          {discoveryError ? (
            <Button
              className="rounded-full px-6"
              onClick={retryDiscovery}
              type="button"
            >
              Retry
            </Button>
          ) : null}
          <Button
            className="rounded-full bg-foreground/10 px-5 hover:bg-foreground/15"
            onClick={onCancel}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  if (!policy) return null;
  const needsAgeConfirmation = policy.ageAttestationRequired;
  const needsAgreement = Boolean(
    policy.termsMarkdown || policy.privacyMarkdown,
  );

  return (
    <div className="flex w-full flex-col items-center">
      <ShieldCheck aria-hidden="true" className="h-10 w-10" />
      <h1
        className="mt-5 text-title font-normal"
        ref={headingRef}
        tabIndex={-1}
      >
        Review community requirements
      </h1>
      <p className="mt-3 max-w-[460px] text-sm leading-6 text-foreground/80">
        Accept the current policy before Buzz uses your saved identity to claim
        this invite.
      </p>
      <div className="mt-6 w-full max-w-[480px]">
        <JoinPolicyNotice
          ageConfirmed={ageConfirmed}
          agreementConfirmed={agreementConfirmed}
          onAgeConfirmedChange={(confirmed) => {
            setAgeConfirmed(confirmed);
            setPolicyError(null);
          }}
          onAgreementConfirmedChange={(confirmed) => {
            setAgreementConfirmed(confirmed);
            setPolicyError(null);
          }}
          policy={policy}
          relayWsUrl={relayUrl ?? ""}
        />
      </div>
      {policyError ? (
        <p
          aria-live="assertive"
          className="mt-4 text-sm text-destructive"
          data-testid="identity-handoff-policy-error"
          role="alert"
        >
          {policyError}
        </p>
      ) : null}
      <div className="mt-6 flex justify-center gap-3">
        <Button
          className="rounded-full px-6"
          disabled={
            isAccepting ||
            (needsAgeConfirmation && !ageConfirmed) ||
            (needsAgreement && !agreementConfirmed)
          }
          onClick={() => void acceptCurrentPolicy()}
          type="button"
        >
          {isAccepting ? (
            <Spinner aria-label="Accepting policy" className="h-4 w-4" />
          ) : (
            "Accept and continue"
          )}
        </Button>
        <Button
          className="rounded-full bg-foreground/10 px-5 hover:bg-foreground/15"
          disabled={isAccepting}
          onClick={onCancel}
          type="button"
          variant="ghost"
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}
