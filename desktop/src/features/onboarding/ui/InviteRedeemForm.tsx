import * as React from "react";

import {
  inviteErrorMessage,
  parseInviteInput,
} from "@/shared/api/inviteHelpers";
import {
  acceptJoinPolicy,
  getJoinPolicy,
  type JoinPolicy,
} from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { JoinPolicyNotice } from "./JoinPolicyNotice";

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
  onRedeem: (relayWsUrl: string, code: string, policyReceipt?: string) => void;
};

export function InviteRedeemForm({
  defaultRelayUrl,
  error,
  isRedeeming,
  onCancel,
  onRedeem,
}: InviteRedeemFormProps) {
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
  const [policyError, setPolicyError] = React.useState<string | null>(null);
  const [isLoadingPolicy, setIsLoadingPolicy] = React.useState(false);

  const parsed = React.useMemo(
    () => parseInviteInput(inviteInput),
    [inviteInput],
  );
  const isBareCode = parsed !== null && !("relayWsUrl" in parsed);
  const needsRelayField = isBareCode && defaultRelayUrl !== undefined;

  const canSubmit =
    parsed !== null &&
    ("relayWsUrl" in parsed ||
      (isBareCode && bareCodeRelayUrl.trim().length > 0));

  const handleSubmit = React.useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();
      if (!parsed) return;

      const relayWsUrl =
        "relayWsUrl" in parsed ? parsed.relayWsUrl : bareCodeRelayUrl.trim();
      if (!relayWsUrl) return;

      setPolicyError(null);
      setIsLoadingPolicy(true);
      try {
        const policy = await getJoinPolicy(relayWsUrl);
        if (!policy) {
          onRedeem(relayWsUrl, parsed.code);
          return;
        }

        if (
          !joinPolicy ||
          joinPolicy.version !== policy.version ||
          policyInvite?.relayWsUrl !== relayWsUrl ||
          policyInvite.code !== parsed.code
        ) {
          setJoinPolicy(policy);
          setPolicyInvite({ relayWsUrl, code: parsed.code });
          setAgeConfirmed(false);
          return;
        }

        if (policy.ageAttestationRequired && !ageConfirmed) {
          setPolicyError("Confirm that you are at least 18 years old.");
          return;
        }

        const receipt = await acceptJoinPolicy(
          relayWsUrl,
          parsed.code,
          policy.version,
          ageConfirmed,
        );
        onRedeem(relayWsUrl, parsed.code, receipt);
      } catch (policyFetchError) {
        setPolicyError(inviteErrorMessage(policyFetchError));
      } finally {
        setIsLoadingPolicy(false);
      }
    },
    [
      ageConfirmed,
      bareCodeRelayUrl,
      joinPolicy,
      onRedeem,
      parsed,
      policyInvite,
    ],
  );

  return (
    <form className="flex w-full flex-col gap-3" onSubmit={handleSubmit}>
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
          onChange={(event) => {
            setInviteInput(event.target.value);
            setJoinPolicy(null);
            setPolicyInvite(null);
            setAgeConfirmed(false);
            setPolicyError(null);
          }}
          placeholder="https://relay.example.com/invite/abc123 or paste a code"
          spellCheck={false}
          type="text"
          value={inviteInput}
        />
      </div>

      {needsRelayField ? (
        <div className="space-y-1.5 text-left">
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
            onChange={(event) => {
              setBareCodeRelayUrl(event.target.value);
              setJoinPolicy(null);
              setPolicyInvite(null);
              setAgeConfirmed(false);
              setPolicyError(null);
            }}
            placeholder="wss://relay.example.com"
            type="text"
            value={bareCodeRelayUrl}
          />
        </div>
      ) : null}

      {joinPolicy && policyInvite ? (
        <JoinPolicyNotice
          ageConfirmed={ageConfirmed}
          onAgeConfirmedChange={(confirmed) => {
            setAgeConfirmed(confirmed);
            setPolicyError(null);
          }}
          policy={joinPolicy}
          relayWsUrl={policyInvite.relayWsUrl}
        />
      ) : null}

      {policyError ? (
        <p className="text-center text-sm text-destructive">{policyError}</p>
      ) : null}

      {error ? (
        <p className="text-center text-sm text-destructive">{error}</p>
      ) : null}

      <Button
        className="h-10 w-full"
        data-testid="invite-redeem-submit"
        disabled={
          !canSubmit ||
          isRedeeming ||
          isLoadingPolicy ||
          Boolean(joinPolicy?.ageAttestationRequired && !ageConfirmed)
        }
        type="submit"
      >
        {isRedeeming || isLoadingPolicy ? (
          <Spinner
            aria-label={isRedeeming ? "Redeeming invite" : "Loading policy"}
            className="h-4 w-4 border-2"
          />
        ) : joinPolicy ? (
          "Accept and redeem invite"
        ) : (
          "Redeem invite"
        )}
      </Button>

      <Button
        className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
        disabled={isRedeeming}
        onClick={onCancel}
        type="button"
        variant="ghost"
      >
        Cancel
      </Button>
    </form>
  );
}
