import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { useTranslation } from "@/i18n";
import { joinPolicyDocumentUrl, type JoinPolicy } from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";

type JoinPolicyNoticeProps = {
  ageConfirmed: boolean;
  agreementConfirmed: boolean;
  onAgeConfirmedChange: (confirmed: boolean) => void;
  onAgreementConfirmedChange: (confirmed: boolean) => void;
  policy: JoinPolicy;
  /** Relay hosting the policy documents the links below point at. */
  relayWsUrl: string;
};

/**
 * Join-policy consent block shown on every join surface.
 *
 * The Terms/Privacy links open the relay-hosted document pages
 * (`/api/join-policy/terms|privacy`) in the system browser via the OS
 * opener. They must NOT navigate or render in-app: these surfaces exist
 * before onboarding completes, where the router (required by the message
 * Markdown component) is not mounted — an in-app render tears down the
 * whole React tree.
 */
export function JoinPolicyNotice({
  ageConfirmed,
  agreementConfirmed,
  onAgeConfirmedChange,
  onAgreementConfirmedChange,
  policy,
  relayWsUrl,
}: JoinPolicyNoticeProps) {
  const ageConfirmationId = React.useId();
  const agreementConfirmationId = React.useId();
  const { t } = useTranslation();

  return (
    <div className="space-y-3 rounded-xl border border-border/70 bg-muted/30 p-4 text-left">
      {policy.ageAttestationRequired ? (
        <div className="flex items-start gap-3">
          <Checkbox
            checked={ageConfirmed}
            className="mt-0.5"
            id={ageConfirmationId}
            onCheckedChange={(checked) =>
              onAgeConfirmedChange(checked === true)
            }
          />
          <label
            className="cursor-pointer text-xs leading-5 text-muted-foreground"
            htmlFor={ageConfirmationId}
          >
            {t("onboarding.membership.age-attestation")}
          </label>
        </div>
      ) : null}

      {policy.termsMarkdown || policy.privacyMarkdown ? (
        <div className="flex items-start gap-3">
          <Checkbox
            checked={agreementConfirmed}
            className="mt-0.5"
            id={agreementConfirmationId}
            onCheckedChange={(checked) =>
              onAgreementConfirmedChange(checked === true)
            }
          />
          <label
            className="cursor-pointer text-xs leading-5 text-muted-foreground"
            htmlFor={agreementConfirmationId}
          >
            {t("onboarding.membership.agree-intro")}
            {policy.termsMarkdown ? (
              <Button
                className="h-auto p-0 align-baseline text-xs no-underline hover:underline focus-visible:no-underline"
                onClick={(event) => {
                  event.preventDefault();
                  void openUrl(joinPolicyDocumentUrl(relayWsUrl, "terms"));
                }}
                type="button"
                variant="link"
              >
                {t("onboarding.membership.terms-link")}
              </Button>
            ) : null}
            {policy.termsMarkdown && policy.privacyMarkdown
              ? t("onboarding.membership.agree-conjunction")
              : null}
            {policy.privacyMarkdown ? (
              <Button
                className="h-auto p-0 align-baseline text-xs no-underline hover:underline focus-visible:no-underline"
                onClick={(event) => {
                  event.preventDefault();
                  void openUrl(joinPolicyDocumentUrl(relayWsUrl, "privacy"));
                }}
                type="button"
                variant="link"
              >
                {t("onboarding.membership.privacy-link")}
              </Button>
            ) : null}
            {t("onboarding.membership.agree-terminal")}
          </label>
        </div>
      ) : null}
    </div>
  );
}
