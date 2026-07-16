import { openUrl } from "@tauri-apps/plugin-opener";

import { joinPolicyDocumentUrl, type JoinPolicy } from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";

type JoinPolicyNoticeProps = {
  ageConfirmed: boolean;
  onAgeConfirmedChange: (confirmed: boolean) => void;
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
  onAgeConfirmedChange,
  policy,
  relayWsUrl,
}: JoinPolicyNoticeProps) {
  return (
    <div className="space-y-3 rounded-md border p-3 text-left text-sm">
      {policy.ageAttestationRequired ? (
        <label className="flex items-start gap-2">
          <input
            checked={ageConfirmed}
            className="mt-1"
            onChange={(event) => onAgeConfirmedChange(event.target.checked)}
            type="checkbox"
          />
          <span>I am 18 years of age or older.</span>
        </label>
      ) : null}

      {policy.termsMarkdown || policy.privacyMarkdown ? (
        <p className="text-muted-foreground">
          By proceeding you agree to the Buzz{" "}
          {policy.termsMarkdown ? (
            <Button
              className="h-auto p-0 align-baseline"
              onClick={() =>
                void openUrl(joinPolicyDocumentUrl(relayWsUrl, "terms"))
              }
              type="button"
              variant="link"
            >
              Terms of Service
            </Button>
          ) : null}
          {policy.termsMarkdown && policy.privacyMarkdown ? " and " : null}
          {policy.privacyMarkdown ? (
            <Button
              className="h-auto p-0 align-baseline"
              onClick={() =>
                void openUrl(joinPolicyDocumentUrl(relayWsUrl, "privacy"))
              }
              type="button"
              variant="link"
            >
              Privacy Policy
            </Button>
          ) : null}
          .
        </p>
      ) : null}
    </div>
  );
}
