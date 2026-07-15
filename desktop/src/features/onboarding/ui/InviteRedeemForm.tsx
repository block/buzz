import * as React from "react";

import { parseInviteInput } from "@/shared/api/inviteHelpers";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

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
  onRedeem: (relayWsUrl: string, code: string) => void;
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
    (event: React.FormEvent) => {
      event.preventDefault();
      if (!parsed) return;

      if ("relayWsUrl" in parsed) {
        onRedeem(parsed.relayWsUrl, parsed.code);
      } else if (bareCodeRelayUrl.trim()) {
        onRedeem(bareCodeRelayUrl.trim(), parsed.code);
      }
    },
    [bareCodeRelayUrl, onRedeem, parsed],
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
          onChange={(event) => setInviteInput(event.target.value)}
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
            onChange={(event) => setBareCodeRelayUrl(event.target.value)}
            placeholder="wss://relay.example.com"
            type="text"
            value={bareCodeRelayUrl}
          />
        </div>
      ) : null}

      {error ? (
        <p className="text-center text-sm text-destructive">{error}</p>
      ) : null}

      <Button
        className="h-10 w-full"
        data-testid="invite-redeem-submit"
        disabled={!canSubmit || isRedeeming}
        type="submit"
      >
        {isRedeeming ? (
          <Spinner aria-label="Redeeming invite" className="h-4 w-4 border-2" />
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
