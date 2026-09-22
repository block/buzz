import * as React from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import {
  getJoinPolicy,
  isJoinPolicyDiscoveryCandidate,
  type JoinPolicy,
} from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { JoinPolicyNotice } from "@/features/onboarding/ui/JoinPolicyNotice";
import { useTranslation } from "@/i18n";
import { normalizeRelayUrl, probeRelayReachable } from "../relayProbe";

const POLICY_DISCOVERY_DELAY_MS = 250;
const POLICY_REVEAL_EASE = [0.23, 1, 0.32, 1] as const;

export type CommunityEditFormProps = {
  cancelLabel?: string;
  initialName: string;
  initialRelayUrl: string;
  isSubmitting?: boolean;
  joinPolicyRequired?: boolean;
  onCancel: () => void;
  onSubmit: (name: string, relayUrl: string) => void;
  submitLabel: string;
};

export function CommunityEditForm({
  cancelLabel,
  initialName,
  initialRelayUrl,
  isSubmitting = false,
  joinPolicyRequired = false,
  onCancel,
  onSubmit,
  submitLabel,
}: CommunityEditFormProps) {
  const { t } = useTranslation();
  const [name, setName] = React.useState(initialName);
  const [relayUrl, setRelayUrl] = React.useState(initialRelayUrl);
  const [error, setError] = React.useState<string | null>(null);
  const [probeWarning, setProbeWarning] = React.useState<string | null>(null);
  const [isProbing, setIsProbing] = React.useState(false);
  const [useAnywayOverride, setUseAnywayOverride] = React.useState(false);
  const [joinPolicy, setJoinPolicy] = React.useState<JoinPolicy | null>(null);
  const [policyRelayUrl, setPolicyRelayUrl] = React.useState<string | null>(
    null,
  );
  const [ageConfirmed, setAgeConfirmed] = React.useState(false);
  const [agreementConfirmed, setAgreementConfirmed] = React.useState(false);
  const shouldReduceMotion = useReducedMotion();

  const cancelRef = React.useRef<(() => void) | null>(null);

  React.useEffect(() => {
    return () => {
      cancelRef.current?.();
    };
  }, []);

  React.useEffect(() => {
    if (!joinPolicyRequired) return;

    const normalizedUrl = normalizeRelayUrl(relayUrl);
    if (!normalizedUrl || !isJoinPolicyDiscoveryCandidate(normalizedUrl)) {
      return;
    }

    let cancelled = false;
    const timeoutId = window.setTimeout(() => {
      void getJoinPolicy(normalizedUrl, "native")
        .then((policy) => {
          if (cancelled || !policy) return;
          setJoinPolicy(policy);
          setPolicyRelayUrl(normalizedUrl);
          setAgeConfirmed(false);
          setAgreementConfirmed(false);
          setError(null);
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
  }, [joinPolicyRequired, relayUrl]);

  const handleSubmit = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmedName = name.trim();
      if (!trimmedName) {
        setError(t("communities.editform.error-name-required"));
        return;
      }
      const normalizedUrl = normalizeRelayUrl(relayUrl);
      if (!normalizedUrl) {
        setError(t("communities.editform.error-url-invalid"));
        return;
      }

      if (joinPolicyRequired) {
        try {
          const policy = await getJoinPolicy(normalizedUrl, "native");
          if (!policy) {
            onSubmit(trimmedName, normalizedUrl);
            return;
          }
          if (
            !joinPolicy ||
            joinPolicy.version !== policy.version ||
            policyRelayUrl !== normalizedUrl
          ) {
            setJoinPolicy(policy);
            setPolicyRelayUrl(normalizedUrl);
            setAgeConfirmed(false);
            setAgreementConfirmed(false);
            return;
          }
          if (policy.ageAttestationRequired && !ageConfirmed) {
            setError(t("communities.editform.error-age"));
            return;
          }
          if (
            (policy.termsMarkdown || policy.privacyMarkdown) &&
            !agreementConfirmed
          ) {
            setError(t("communities.editform.error-agreement"));
            return;
          }
        } catch (policyError) {
          setError(inviteErrorMessage(policyError));
          return;
        }
      }

      if (useAnywayOverride) {
        onSubmit(trimmedName, normalizedUrl);
        return;
      }

      // Cancel any in-flight probe before starting a new one.
      cancelRef.current?.();
      setIsProbing(true);
      setProbeWarning(null);

      const probe = probeRelayReachable(normalizedUrl);
      cancelRef.current = probe.cancel;

      let reachable: boolean;
      try {
        reachable = await probe.promise;
      } finally {
        setIsProbing(false);
      }

      if (!reachable) {
        setProbeWarning(t("communities.editform.warning-relay-unreachable"));
        return;
      }

      onSubmit(trimmedName, normalizedUrl);
    },
    [
      ageConfirmed,
      agreementConfirmed,
      joinPolicy,
      joinPolicyRequired,
      name,
      onSubmit,
      policyRelayUrl,
      relayUrl,
      t,
      useAnywayOverride,
    ],
  );

  const handleUseAnyway = React.useCallback(() => {
    setUseAnywayOverride(true);
    setProbeWarning(null);
    const trimmedName = name.trim();
    const normalizedUrl = normalizeRelayUrl(relayUrl);
    if (trimmedName && normalizedUrl) {
      onSubmit(trimmedName, normalizedUrl);
    }
  }, [name, onSubmit, relayUrl]);

  const isBusy = isSubmitting || isProbing;

  return (
    <form
      className="flex w-full flex-col gap-4"
      onSubmit={(e) => {
        void handleSubmit(e);
      }}
    >
      <div className="space-y-1.5 text-left">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="community-edit-name"
        >
          {t("communities.editform.name-label")}
        </label>
        <Input
          autoFocus
          className="h-10 bg-background"
          disabled={isProbing}
          id="community-edit-name"
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
          placeholder={t("communities.editform.name-placeholder")}
          type="text"
          value={name}
        />
      </div>

      <div className="space-y-1.5 text-left">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="community-edit-url"
        >
          {t("communities.editform.url-label")}
        </label>
        <Input
          className="h-10 bg-background"
          disabled={isProbing}
          id="community-edit-url"
          onChange={(event) => {
            setRelayUrl(event.target.value);
            setError(null);
            setProbeWarning(null);
            setUseAnywayOverride(false);
            setJoinPolicy(null);
            setPolicyRelayUrl(null);
            setAgeConfirmed(false);
            setAgreementConfirmed(false);
          }}
          placeholder="wss://relay.example.com"
          type="text"
          value={relayUrl}
        />
      </div>

      <AnimatePresence initial={false}>
        {joinPolicy && policyRelayUrl ? (
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
                ? { height: 0, marginTop: "-1rem", opacity: 0 }
                : {
                    height: 0,
                    marginTop: "-1rem",
                    opacity: 0,
                    transform: "translateY(-0.25rem)",
                  }
            }
            initial={
              shouldReduceMotion
                ? false
                : {
                    height: 0,
                    marginTop: "-1rem",
                    opacity: 0,
                    transform: "translateY(-0.25rem)",
                  }
            }
            key={`${policyRelayUrl}:${joinPolicy.version}`}
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
                setError(null);
              }}
              onAgreementConfirmedChange={(confirmed) => {
                setAgreementConfirmed(confirmed);
                setError(null);
              }}
              policy={joinPolicy}
              relayWsUrl={policyRelayUrl}
            />
          </motion.div>
        ) : null}
      </AnimatePresence>

      <div className="flex w-full flex-col gap-3 pt-1">
        <Button
          className="h-10 w-full"
          disabled={
            isBusy ||
            !name.trim() ||
            !relayUrl.trim() ||
            Boolean(joinPolicy?.ageAttestationRequired && !ageConfirmed) ||
            Boolean(
              joinPolicy &&
                (joinPolicy.termsMarkdown || joinPolicy.privacyMarkdown) &&
                !agreementConfirmed,
            )
          }
          type="submit"
        >
          {isProbing ? (
            <Spinner
              aria-label={t("communities.editform.checking-aria")}
              className="h-4 w-4 border-2"
            />
          ) : isSubmitting ? (
            <Spinner
              aria-label={t("communities.editform.saving-aria")}
              className="h-4 w-4 border-2"
            />
          ) : (
            submitLabel
          )}
        </Button>

        <Button
          className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
          disabled={isBusy}
          onClick={onCancel}
          type="button"
          variant="ghost"
        >
          {cancelLabel ?? t("communities.editform.cancel")}
        </Button>

        {error ? (
          <p className="text-center text-sm text-destructive">{error}</p>
        ) : null}

        {probeWarning ? (
          <div className="flex flex-col items-center gap-2">
            <p className="text-center text-sm text-destructive">
              {probeWarning}
            </p>
            <Button
              className="h-8 text-xs"
              onClick={handleUseAnyway}
              size="sm"
              type="button"
              variant="outline"
            >
              {t("communities.editform.use-anyway")}
            </Button>
          </div>
        ) : null}
      </div>
    </form>
  );
}
