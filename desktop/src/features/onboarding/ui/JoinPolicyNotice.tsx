import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { joinPolicyDocumentUrl, type JoinPolicy } from "@/shared/api/invites";
import { cn } from "@/shared/lib/cn";
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
  /** Optional safe host for surfaces that must not open external documents. */
  onOpenDocument?: (document: "terms" | "privacy") => void;
  /** Use primary page text where a surface needs stronger consent contrast. */
  textTone?: "muted" | "foreground";
};

/**
 * Join-policy consent block shown on every join surface.
 *
 * The Terms/Privacy links normally open the relay-hosted document pages
 * (`/api/join-policy/terms|privacy`) in the system browser via the OS opener.
 * A non-networked host may intercept those actions. They must NOT navigate or
 * render in-app: these surfaces exist before onboarding completes, where the
 * router (required by the message Markdown component) is not mounted — an
 * in-app render tears down the whole React tree.
 */
export function JoinPolicyNotice({
  ageConfirmed,
  agreementConfirmed,
  onAgeConfirmedChange,
  onAgreementConfirmedChange,
  onOpenDocument,
  policy,
  relayWsUrl,
  textTone = "muted",
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
            className={cn(
              "cursor-pointer text-xs leading-5",
              textTone === "foreground"
                ? "text-foreground"
                : "text-muted-foreground",
            )}
            htmlFor={ageConfirmationId}
          >
            I am 18 years of age or older.
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
            className={cn(
              "cursor-pointer text-xs leading-5",
              textTone === "foreground"
                ? "text-foreground"
                : "text-muted-foreground",
            )}
            htmlFor={agreementConfirmationId}
          >
            I agree to the Buzz{" "}
            {policy.termsMarkdown ? (
              <Button
                className={cn(
                  "h-auto p-0 align-baseline text-xs",
                  textTone === "foreground"
                    ? "font-medium text-foreground underline hover:underline focus-visible:underline"
                    : "no-underline hover:underline focus-visible:no-underline",
                )}
                onClick={(event) => {
                  event.preventDefault();
                  if (onOpenDocument) {
                    onOpenDocument("terms");
                  } else {
                    void openUrl(joinPolicyDocumentUrl(relayWsUrl, "terms"));
                  }
                }}
                type="button"
                variant="link"
              >
                Terms of Service
              </Button>
            ) : null}
            {policy.termsMarkdown && policy.privacyMarkdown ? " and " : null}
            {policy.privacyMarkdown ? (
              <Button
                className={cn(
                  "h-auto p-0 align-baseline text-xs",
                  textTone === "foreground"
                    ? "font-medium text-foreground underline hover:underline focus-visible:underline"
                    : "no-underline hover:underline focus-visible:no-underline",
                )}
                onClick={(event) => {
                  event.preventDefault();
                  if (onOpenDocument) {
                    onOpenDocument("privacy");
                  } else {
                    void openUrl(joinPolicyDocumentUrl(relayWsUrl, "privacy"));
                  }
                }}
                type="button"
                variant="link"
              >
                Privacy Policy
              </Button>
            ) : null}
            .
          </label>
        </div>
      ) : null}
    </div>
  );
}
