import * as React from "react";

import { parsePubkeyInput } from "@/features/agents/lib/respondToAllowlist";
import type { RespondToMode } from "@/shared/api/types";
import type {
  ExistingAgentRegistrationResult,
  RegisterExistingAgentInput,
} from "@/shared/api/tauriAgentRegistration";
import { Button } from "@/shared/ui/button";

const HEX_PUBLIC_KEY_PATTERN = /^[0-9a-fA-F]{64}$/;

export function existingAgentRegistrationMessage(
  result: ExistingAgentRegistrationResult,
) {
  if (result.alreadyRegistered) {
    return `${result.displayName} is already registered.`;
  }
  if (result.publicationStatus === "queued") {
    return `Registration for ${result.displayName} is queued and will publish when the relay is reachable.`;
  }
  return `${result.displayName} is registered and can appear in mention suggestions in channels where it is a bot member.`;
}

export function RegisterExistingAgentPane({
  isPending,
  onRegister,
}: {
  isPending: boolean;
  onRegister: (
    input: RegisterExistingAgentInput,
  ) => Promise<ExistingAgentRegistrationResult>;
}) {
  const [agentPubkey, setAgentPubkey] = React.useState("");
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [allowlistInput, setAllowlistInput] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] =
    React.useState<ExistingAgentRegistrationResult | null>(null);
  const normalizedPubkey = agentPubkey.trim();
  const parsedAllowlist = React.useMemo(
    () => parsePubkeyInput(allowlistInput),
    [allowlistInput],
  );
  const allowlistIsValid =
    respondTo !== "allowlist" ||
    (parsedAllowlist.valid.length > 0 && parsedAllowlist.invalid.length === 0);
  const canSubmit =
    HEX_PUBLIC_KEY_PATTERN.test(normalizedPubkey) &&
    allowlistIsValid &&
    !isPending;

  const policySummary =
    respondTo === "owner-only"
      ? "Only you can find this agent in mention suggestions."
      : respondTo === "allowlist"
        ? `${parsedAllowlist.valid.length} selected ${parsedAllowlist.valid.length === 1 ? "person" : "people"} can find this agent in mention suggestions.`
        : "Anyone who shares a channel with this agent can find it in mention suggestions.";

  return (
    <form
      className="flex min-h-0 flex-1 flex-col overflow-y-auto px-6 py-6"
      data-testid="register-existing-agent-pane"
      onSubmit={(event) => {
        event.preventDefault();
        if (!canSubmit) return;
        setError(null);
        setResult(null);
        void onRegister({
          agentPubkey: normalizedPubkey,
          respondTo,
          respondToAllowlist:
            respondTo === "allowlist" ? parsedAllowlist.valid : [],
        })
          .then(setResult)
          .catch((cause: unknown) => {
            setError(
              cause instanceof Error
                ? cause.message
                : "Could not register this agent.",
            );
          });
      }}
    >
      <div className="max-w-xl space-y-5">
        <div>
          <h3 className="text-lg font-semibold">Register an existing agent</h3>
          <p className="mt-1 text-sm text-muted-foreground">
            Connect an agent that already has its own key and runtime. Its
            published profile must prove that you are the owner.
          </p>
        </div>

        <label className="block space-y-2">
          <span className="text-sm font-medium">Agent public key</span>
          <input
            autoComplete="off"
            className="h-10 w-full rounded-md border border-input bg-background px-3 font-mono text-sm outline-hidden transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="register-existing-agent-pubkey"
            disabled={isPending}
            onChange={(event) => setAgentPubkey(event.target.value)}
            placeholder="64-character hex public key"
            spellCheck={false}
            value={agentPubkey}
          />
        </label>

        <label className="block space-y-2">
          <span className="text-sm font-medium">
            Who can find and mention this agent
          </span>
          <select
            className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="register-existing-agent-respond-to"
            disabled={isPending}
            onChange={(event) => {
              setRespondTo(event.target.value as RespondToMode);
              setError(null);
              setResult(null);
            }}
            value={respondTo}
          >
            <option value="owner-only">Only me (default)</option>
            <option value="allowlist">Selected people</option>
            <option value="anyone">Anyone in a shared channel</option>
          </select>
        </label>

        {respondTo === "allowlist" ? (
          <label className="block space-y-2">
            <span className="text-sm font-medium">
              People&apos;s public keys
            </span>
            <textarea
              className="min-h-24 w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm outline-hidden transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="register-existing-agent-allowlist"
              disabled={isPending}
              onChange={(event) => {
                setAllowlistInput(event.target.value);
                setError(null);
                setResult(null);
              }}
              placeholder="One 64-character public key per line"
              spellCheck={false}
              value={allowlistInput}
            />
            <p className="text-xs text-muted-foreground">
              Include your own public key too if you want this agent in your
              mention suggestions.
            </p>
            {parsedAllowlist.invalid.length > 0 ? (
              <p className="text-xs text-destructive" role="alert">
                Every entry must be a 64-character hex public key.
              </p>
            ) : null}
          </label>
        ) : null}

        <div className="rounded-lg border bg-muted/30 px-4 py-3 text-sm">
          <p
            className="font-medium"
            data-testid="register-existing-agent-policy-summary"
          >
            {policySummary}
          </p>
          <p className="mt-1 text-muted-foreground">
            Buzz publishes an owner-signed directory record. It does not
            generate, import, replace, or store the agent&apos;s private key.
          </p>
          <p className="mt-1 text-muted-foreground">
            This setting controls Desktop discovery only. Configure the external
            agent runtime to accept the same people.
          </p>
        </div>

        {error ? (
          <p
            className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
            role="alert"
          >
            {error}
          </p>
        ) : null}
        {result ? (
          <p
            aria-live="polite"
            className="rounded-lg border border-border bg-muted/30 px-4 py-3 text-sm"
            data-testid="register-existing-agent-result"
          >
            {existingAgentRegistrationMessage(result)}
          </p>
        ) : null}

        <Button
          data-testid="register-existing-agent-submit"
          disabled={!canSubmit}
          type="submit"
        >
          {isPending ? "Registering…" : "Register agent"}
        </Button>
      </div>
    </form>
  );
}
