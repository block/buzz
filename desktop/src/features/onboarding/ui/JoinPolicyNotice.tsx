import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

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
 * The Content Guidelines, Terms, and Privacy links open the relay-hosted
 * document pages (`/api/join-policy/content-guidelines|terms|privacy`) in the
 * system browser via the OS
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
            I am 18 years of age or older.
          </label>
        </div>
      ) : null}

      {policy.contentGuidelinesMarkdown ||
      policy.termsMarkdown ||
      policy.privacyMarkdown ? (
        <div className="flex items-start gap-3">
          <Checkbox
            aria-label="I have read the Content Guidelines and agree to the Buzz Terms of Service and Privacy Policy."
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
            I have read the{" "}
            {policy.contentGuidelinesMarkdown ? (
              <Button
                className="h-auto p-0 align-baseline text-xs no-underline hover:underline focus-visible:no-underline"
                onClick={(event) => {
                  event.preventDefault();
                  void openUrl(
                    joinPolicyDocumentUrl(relayWsUrl, "content-guidelines"),
                  );
                }}
                type="button"
                variant="link"
              >
                Content Guidelines
              </Button>
            ) : (
              "Content Guidelines"
            )}{" "}
            and agree to the Buzz{" "}
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
                Terms of Service
              </Button>
            ) : (
              "Terms of Service"
            )}{" "}
            and{" "}
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
                Privacy Policy
              </Button>
            ) : (
              "Privacy Policy"
            )}
            .
          </label>
        </div>
      ) : null}
    </div>
  );
}
