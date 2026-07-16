import * as React from "react";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import { getJoinPolicy, type JoinPolicy } from "@/shared/api/invites";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { JoinPolicyNotice } from "@/features/onboarding/ui/JoinPolicyNotice";
import { normalizeRelayUrl, probeRelayReachable } from "../relayProbe";

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
  cancelLabel = "Cancel",
  initialName,
  initialRelayUrl,
  isSubmitting = false,
  joinPolicyRequired = false,
  onCancel,
  onSubmit,
  submitLabel,
}: CommunityEditFormProps) {
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

  const cancelRef = React.useRef<(() => void) | null>(null);

  React.useEffect(() => {
    return () => {
      cancelRef.current?.();
    };
  }, []);

  const handleSubmit = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmedName = name.trim();
      if (!trimmedName) {
        setError("Please enter a community name.");
        return;
      }
      const normalizedUrl = normalizeRelayUrl(relayUrl);
      if (!normalizedUrl) {
        setError("Enter a valid ws:// or wss:// relay URL.");
        return;
      }

      if (joinPolicyRequired) {
        try {
          const policy = await getJoinPolicy(normalizedUrl);
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
            return;
          }
          if (policy.ageAttestationRequired && !ageConfirmed) {
            setError("Confirm that you are at least 18 years old.");
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
        setProbeWarning("Can't reach this relay — check the URL");
        return;
      }

      onSubmit(trimmedName, normalizedUrl);
    },
    [
      ageConfirmed,
      joinPolicy,
      joinPolicyRequired,
      name,
      onSubmit,
      policyRelayUrl,
      relayUrl,
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
          Community name
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
          placeholder="Design team"
          type="text"
          value={name}
        />
      </div>

      <div className="space-y-1.5 text-left">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="community-edit-url"
        >
          Community URL
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
          }}
          placeholder="wss://relay.example.com"
          type="text"
          value={relayUrl}
        />
      </div>

      {joinPolicy && policyRelayUrl ? (
        <JoinPolicyNotice
          ageConfirmed={ageConfirmed}
          onAgeConfirmedChange={(confirmed) => {
            setAgeConfirmed(confirmed);
            setError(null);
          }}
          policy={joinPolicy}
          relayWsUrl={policyRelayUrl}
        />
      ) : null}

      <div className="flex w-full flex-col gap-3 pt-1">
        <Button
          className="h-10 w-full"
          disabled={
            isBusy ||
            !name.trim() ||
            !relayUrl.trim() ||
            Boolean(joinPolicy?.ageAttestationRequired && !ageConfirmed)
          }
          type="submit"
        >
          {isProbing ? (
            <Spinner aria-label="Checking relay" className="h-4 w-4 border-2" />
          ) : isSubmitting ? (
            <Spinner aria-label="Saving" className="h-4 w-4 border-2" />
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
          {cancelLabel}
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
              Use anyway
            </Button>
          </div>
        ) : null}
      </div>
    </form>
  );
}
