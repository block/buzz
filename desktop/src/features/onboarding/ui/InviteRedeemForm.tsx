import * as React from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import {
  inviteErrorMessage,
  parseInviteInput,
} from "@/shared/api/inviteHelpers";
import {
  acceptJoinPolicy,
  getJoinPolicy,
  isJoinPolicyDiscoveryCandidate,
  type JoinPolicy,
} from "@/shared/api/invites";
import { normalizeRelayUrl } from "@/features/communities/relayProbe";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { JoinPolicyNotice } from "./JoinPolicyNotice";
import {
  ONBOARDING_KEY_ROW_CLASS,
  ONBOARDING_KEY_TEXT_CLASS,
} from "./NsecMaskedDisplay";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";

const POLICY_DISCOVERY_DELAY_MS = 250;
const POLICY_REVEAL_EASE = [0.23, 1, 0.32, 1] as const;
const SPOTLIGHT_TEXTURE_CONTENT_CLASS = "mx-auto w-full max-w-[920px]";
const SPOTLIGHT_OVERFLOW_FADE = {
  WebkitMaskImage:
    "linear-gradient(to right, transparent, black 2rem, black calc(100% - 2rem), transparent)",
  maskImage:
    "linear-gradient(to right, transparent, black 2rem, black calc(100% - 2rem), transparent)",
};

type InviteRedeemFormProps = {
  /**
   * Pre-fill and expose a relay URL field for bare-code inputs.
   * On MembershipDenied this is the active relay; on WelcomeSetup it is the
   * configured default relay.  Omit to silently reject bare-code inputs
   * (the form stays invalid until a full invite URL is entered).
   */
  defaultRelayUrl?: string;
  error: string | null;
  isRedeeming: boolean;
  onCancel: () => void;
  onConnect?: (relayWsUrl: string) => void;
  onRedeem: (relayWsUrl: string, code: string, policyReceipt?: string) => void;
  placeholder?: string;
  variant?: "default" | "onboarding-spotlight";
};

export function InviteRedeemForm({
  defaultRelayUrl,
  error,
  isRedeeming,
  onCancel,
  onConnect,
  onRedeem,
  placeholder,
  variant = "default",
}: InviteRedeemFormProps) {
  const formId = React.useId();
  const [inviteInput, setInviteInput] = React.useState("");
  const [bareCodeRelayUrl, setBareCodeRelayUrl] = React.useState(
    defaultRelayUrl ?? "",
  );
  const [joinPolicy, setJoinPolicy] = React.useState<JoinPolicy | null>(null);
  const [policyInvite, setPolicyInvite] = React.useState<{
    relayWsUrl: string;
    code: string;
  } | null>(null);
  const [ageConfirmed, setAgeConfirmed] = React.useState(false);
  const [agreementConfirmed, setAgreementConfirmed] = React.useState(false);
  const [policyError, setPolicyError] = React.useState<string | null>(null);
  const [isLoadingPolicy, setIsLoadingPolicy] = React.useState(false);
  const shouldReduceMotion = useReducedMotion();

  const parsed = React.useMemo(
    () => parseInviteInput(inviteInput),
    [inviteInput],
  );
  const normalizedRelayUrl = React.useMemo(
    () => (onConnect && !parsed ? normalizeRelayUrl(inviteInput) : null),
    [inviteInput, onConnect, parsed],
  );
  const parsedInvite = parsed;
  const isBareCode = parsedInvite !== null && !("relayWsUrl" in parsedInvite);
  const needsRelayField = isBareCode && defaultRelayUrl !== undefined;

  React.useEffect(() => {
    if (!parsedInvite) return;

    const relayWsUrl =
      "relayWsUrl" in parsedInvite &&
      typeof parsedInvite.relayWsUrl === "string"
        ? parsedInvite.relayWsUrl
        : bareCodeRelayUrl.trim();
    if (!relayWsUrl || !isJoinPolicyDiscoveryCandidate(relayWsUrl)) return;

    let cancelled = false;
    const timeoutId = window.setTimeout(() => {
      void getJoinPolicy(relayWsUrl)
        .then((policy) => {
          if (cancelled || !policy) return;
          setJoinPolicy(policy);
          setPolicyInvite({ relayWsUrl, code: parsedInvite.code });
          setAgeConfirmed(false);
          setAgreementConfirmed(false);
          setPolicyError(null);
        })
        .catch(() => {
          // Background discovery is best-effort. A deliberate submit retries
          // the request and surfaces any relay error to the user.
        });
    }, POLICY_DISCOVERY_DELAY_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [bareCodeRelayUrl, parsedInvite]);

  const canSubmit =
    (parsedInvite !== null &&
      ("relayWsUrl" in parsedInvite ||
        (isBareCode && bareCodeRelayUrl.trim().length > 0))) ||
    normalizedRelayUrl !== null;
  const isOnboardingSpotlight = variant === "onboarding-spotlight";
  const showInvalidInviteTip =
    isOnboardingSpotlight && inviteInput.trim().length > 0 && !canSubmit;

  const handleSubmit = React.useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();
      if (normalizedRelayUrl) {
        onConnect?.(normalizedRelayUrl);
        return;
      }
      if (!parsedInvite) return;

      const relayWsUrl =
        "relayWsUrl" in parsedInvite
          ? parsedInvite.relayWsUrl
          : bareCodeRelayUrl.trim();
      if (!relayWsUrl) return;

      setPolicyError(null);
      setIsLoadingPolicy(true);
      try {
        const policy = await getJoinPolicy(relayWsUrl);
        if (!policy) {
          onRedeem(relayWsUrl, parsedInvite.code);
          return;
        }

        if (
          !joinPolicy ||
          joinPolicy.version !== policy.version ||
          policyInvite?.relayWsUrl !== relayWsUrl ||
          policyInvite.code !== parsedInvite.code
        ) {
          setJoinPolicy(policy);
          setPolicyInvite({ relayWsUrl, code: parsedInvite.code });
          setAgeConfirmed(false);
          setAgreementConfirmed(false);
          return;
        }

        if (policy.ageAttestationRequired && !ageConfirmed) {
          setPolicyError("Confirm that you are at least 18 years old.");
          return;
        }
        if (
          (policy.termsMarkdown || policy.privacyMarkdown) &&
          !agreementConfirmed
        ) {
          setPolicyError("Agree to the Terms of Service and Privacy Policy.");
          return;
        }

        const receipt = await acceptJoinPolicy(
          relayWsUrl,
          parsedInvite.code,
          policy.version,
          ageConfirmed,
        );
        onRedeem(relayWsUrl, parsedInvite.code, receipt);
      } catch (policyFetchError) {
        setPolicyError(inviteErrorMessage(policyFetchError));
      } finally {
        setIsLoadingPolicy(false);
      }
    },
    [
      ageConfirmed,
      agreementConfirmed,
      bareCodeRelayUrl,
      joinPolicy,
      onRedeem,
      normalizedRelayUrl,
      onConnect,
      parsedInvite,
      policyInvite,
    ],
  );

  const handleInviteInputChange = (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    setInviteInput(event.target.value);
    setJoinPolicy(null);
    setPolicyInvite(null);
    setAgeConfirmed(false);
    setAgreementConfirmed(false);
    setPolicyError(null);
  };

  const handleRelayInputChange = (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    setBareCodeRelayUrl(event.target.value);
    setJoinPolicy(null);
    setPolicyInvite(null);
    setAgeConfirmed(false);
    setAgreementConfirmed(false);
    setPolicyError(null);
  };

  const submitButton = (
    <Button
      className={
        isOnboardingSpotlight ? ONBOARDING_PRIMARY_CTA_CLASS : "h-10 w-full"
      }
      data-testid="invite-redeem-submit"
      disabled={
        !canSubmit ||
        isRedeeming ||
        isLoadingPolicy ||
        Boolean(joinPolicy?.ageAttestationRequired && !ageConfirmed) ||
        Boolean(
          joinPolicy &&
            (joinPolicy.termsMarkdown || joinPolicy.privacyMarkdown) &&
            !agreementConfirmed,
        )
      }
      form={formId}
      type="submit"
    >
      {isRedeeming || isLoadingPolicy ? (
        <Spinner
          aria-label={isRedeeming ? "Redeeming invite" : "Loading policy"}
          className="h-4 w-4 border-2"
        />
      ) : isOnboardingSpotlight ? (
        "Next"
      ) : joinPolicy ? (
        "Accept and redeem invite"
      ) : (
        "Redeem invite"
      )}
    </Button>
  );

  const cancelButton = (
    <Button
      className={
        isOnboardingSpotlight
          ? "h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
          : "h-10 w-full text-muted-foreground hover:text-accent-foreground"
      }
      disabled={isRedeeming}
      onClick={onCancel}
      type="button"
      variant="ghost"
    >
      {isOnboardingSpotlight ? "Back" : "Cancel"}
    </Button>
  );

  return (
    <form
      className={cn(
        "flex w-full flex-col",
        isOnboardingSpotlight ? "relative items-center" : "gap-3",
      )}
      id={formId}
      onSubmit={handleSubmit}
    >
      {isOnboardingSpotlight ? (
        <Card
          className="w-[min(calc(100%+12rem),calc(100vw-2rem))] max-w-[1120px] translate-y-8 px-8 py-6"
          data-testid="invite-redeem-input-frame"
          variant="textured"
        >
          <div
            className={SPOTLIGHT_TEXTURE_CONTENT_CLASS}
            style={SPOTLIGHT_OVERFLOW_FADE}
          >
            <label className="block w-full" htmlFor="invite-input">
              <span className="sr-only">Invite link or code</span>
              <span className={ONBOARDING_KEY_ROW_CLASS}>
                <input
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  className={cn(
                    ONBOARDING_KEY_TEXT_CLASS,
                    "block border-0 bg-transparent p-0 text-center shadow-none outline-none placeholder:text-[var(--buzz-onboarding-backup-ink)] placeholder:opacity-40 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50",
                  )}
                  data-testid="invite-redeem-input"
                  disabled={isRedeeming}
                  id="invite-input"
                  onChange={handleInviteInputChange}
                  placeholder={
                    placeholder ?? "https://relay.example.com/invite/abc123"
                  }
                  spellCheck={false}
                  type="text"
                  value={inviteInput}
                />
              </span>
            </label>
          </div>
        </Card>
      ) : (
        <div className="space-y-1.5 text-left">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="invite-input"
          >
            Invite link or code
          </label>
          <Input
            autoComplete="off"
            autoCorrect="off"
            autoFocus
            className="h-10 bg-background"
            data-testid="invite-redeem-input"
            disabled={isRedeeming}
            id="invite-input"
            onChange={handleInviteInputChange}
            placeholder="https://relay.example.com/invite/abc123 or paste a code"
            spellCheck={false}
            type="text"
            value={inviteInput}
          />
        </div>
      )}

      {isOnboardingSpotlight ? (
        <p
          aria-hidden={!showInvalidInviteTip}
          aria-live="polite"
          className={cn(
            "absolute top-[calc(100%+2rem)] mt-4 min-h-5 w-full max-w-4xl text-center text-sm text-[#717106] transition-opacity duration-150 ease-out",
            showInvalidInviteTip ? "opacity-100" : "opacity-0",
          )}
          data-testid="invalid-invite-tip"
        >
          Please enter a valid invite link or community URL
        </p>
      ) : null}

      {needsRelayField ? (
        <div
          className={cn(
            "space-y-1.5 text-left",
            isOnboardingSpotlight && "w-full max-w-[500px]",
          )}
        >
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="invite-relay-url"
          >
            Relay URL
          </label>
          <Input
            className="h-10 bg-background"
            disabled={isRedeeming}
            id="invite-relay-url"
            onChange={handleRelayInputChange}
            placeholder="wss://relay.example.com"
            type="text"
            value={bareCodeRelayUrl}
          />
        </div>
      ) : null}

      {policyError ? (
        <p className="text-center text-sm text-destructive">{policyError}</p>
      ) : null}

      {error ? (
        <p className="text-center text-sm text-destructive">{error}</p>
      ) : null}

      <AnimatePresence initial={false}>
        {joinPolicy && policyInvite ? (
          <motion.div
            animate={{
              height: "auto",
              marginTop: 0,
              opacity: 1,
              transform: "translateY(0rem)",
            }}
            className="overflow-hidden"
            exit={
              shouldReduceMotion
                ? { height: 0, marginTop: "-0.75rem", opacity: 0 }
                : {
                    height: 0,
                    marginTop: "-0.75rem",
                    opacity: 0,
                    transform: "translateY(-0.25rem)",
                  }
            }
            initial={
              shouldReduceMotion
                ? false
                : {
                    height: 0,
                    marginTop: "-0.75rem",
                    opacity: 0,
                    transform: "translateY(-0.25rem)",
                  }
            }
            key={`${policyInvite.relayWsUrl}:${joinPolicy.version}`}
            transition={
              shouldReduceMotion
                ? { duration: 0 }
                : { duration: 0.22, ease: POLICY_REVEAL_EASE }
            }
          >
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
              policy={joinPolicy}
              relayWsUrl={policyInvite.relayWsUrl}
            />
          </motion.div>
        ) : null}
      </AnimatePresence>

      {isOnboardingSpotlight ? (
        <OnboardingFooter>
          {submitButton}
          {cancelButton}
        </OnboardingFooter>
      ) : (
        <>
          {submitButton}
          {cancelButton}
        </>
      )}
    </form>
  );
}
