import * as React from "react";
import { AlertCircle, ExternalLink, LoaderCircle } from "lucide-react";

import {
  bindBuilderlabIdentity,
  cancelBuilderlabLogin,
  checkHostedCommunityName,
  clearBuilderlabAuth,
  createHostedCommunity,
  deleteBuilderlabIdentity,
  getBuilderlabAuth,
  HOSTED_COMMUNITY_LIMIT,
  HOSTED_COMMUNITY_SUFFIX,
  hostedCommunityErrorMessage,
  hostedCommunityRelayUrl,
  type BuilderlabAuth,
  type HostedCommunity,
  type HostedNostrIdentity,
  loadHostedCommunityAccount,
  startBuilderlabLogin,
  VALID_HOSTED_COMMUNITY_NAME,
} from "@/features/communities/hostedCommunityApi";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "@/features/channels/ui/channelFormStyles";
import { useIdentityQuery } from "@/shared/api/hooks";
import { cn } from "@/shared/lib/cn";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

type HostedCommunityCreateFlowProps = {
  onComplete: () => void;
};

export function HostedCommunityCreateFlow({
  onComplete,
}: HostedCommunityCreateFlowProps) {
  const onboarding = useCommunityOnboarding();
  const localPubkey = useIdentityQuery().data?.pubkey ?? null;
  const [auth, setAuth] = React.useState<BuilderlabAuth | null>(null);
  const [identity, setIdentity] = React.useState<HostedNostrIdentity | null>(
    null,
  );
  const [communities, setCommunities] = React.useState<HostedCommunity[]>([]);
  const [name, setName] = React.useState("");
  const [availability, setAvailability] = React.useState<boolean | null>(null);
  const [checkingName, setCheckingName] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const [action, setAction] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const loginAttempt = React.useRef(0);
  const signingIn = React.useRef(false);

  const loadAccount = React.useCallback(async () => {
    const account = await loadHostedCommunityAccount();
    setIdentity(account.identity);
    setCommunities(account.communities);
  }, []);

  React.useEffect(() => {
    let active = true;
    void getBuilderlabAuth()
      .then(async (nextAuth) => {
        if (!active) return;
        setAuth(nextAuth);
        if (nextAuth) await loadAccount();
      })
      .catch((cause) => {
        if (active)
          setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
      loginAttempt.current += 1;
      if (signingIn.current) {
        signingIn.current = false;
        void cancelBuilderlabLogin().catch(() => {
          // The browser sign-in is already detached from this flow.
          // Cancellation is best-effort cleanup for the native callback.
        });
      }
    };
  }, [loadAccount]);

  const run = async (label: string, operation: () => Promise<void>) => {
    setAction(label);
    setError(null);
    try {
      await operation();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setAction(null);
    }
  };

  const signIn = () => {
    const attempt = ++loginAttempt.current;
    signingIn.current = true;
    setAction("Signing in…");
    setError(null);
    void startBuilderlabLogin()
      .then(async (nextAuth) => {
        if (loginAttempt.current !== attempt) return;
        setAuth(nextAuth);
        await loadAccount();
      })
      .catch((cause) => {
        if (loginAttempt.current !== attempt) return;
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (loginAttempt.current === attempt) {
          signingIn.current = false;
          setAction(null);
        }
      });
  };

  const connectIdentity = () =>
    run("Connecting identity…", async () => {
      const response = await bindBuilderlabIdentity();
      if (response.error) {
        throw new Error(
          hostedCommunityErrorMessage(
            response.error,
            response.correlation_id,
            "Could not connect the Buzz identity.",
          ),
        );
      }
      setIdentity(response.identity ?? null);
      await loadAccount();
    });

  const signOut = () =>
    run("Signing out…", async () => {
      await clearBuilderlabAuth();
      setAuth(null);
      setIdentity(null);
      setCommunities([]);
    });

  const boundPubkey = identity?.pubkey_hex ?? null;
  const identityMismatch = Boolean(
    identity &&
      boundPubkey &&
      localPubkey &&
      boundPubkey.toLowerCase() !== localPubkey.toLowerCase(),
  );
  const localNpub = localPubkey ? safeNpub(localPubkey) : null;

  const switchToDeviceIdentity = () =>
    run("Switching identity…", async () => {
      const released = await deleteBuilderlabIdentity();
      if (released.error) {
        throw new Error(
          hostedCommunityErrorMessage(
            released.error,
            released.correlation_id,
            "Could not disconnect the account's previous Buzz identity.",
          ),
        );
      }
      const bound = await bindBuilderlabIdentity();
      if (bound.error) {
        await loadAccount();
        throw new Error(
          bound.error.code === "pubkey_already_bound"
            ? "This device's Buzz identity belongs to a different Builderlab account. Sign in with the account that already owns this identity."
            : hostedCommunityErrorMessage(
                bound.error,
                bound.correlation_id,
                "Could not connect this device's Buzz identity.",
              ),
        );
      }
      setIdentity(bound.identity ?? null);
      await loadAccount();
    });

  const normalizedName = name.trim().toLowerCase();
  const validName =
    normalizedName.length <= 63 &&
    VALID_HOSTED_COMMUNITY_NAME.test(normalizedName);
  const atCommunityLimit = communities.length >= HOSTED_COMMUNITY_LIMIT;
  const ready = Boolean(auth && identity && !identityMismatch);

  React.useEffect(() => {
    if (!ready || !normalizedName || !validName) {
      setCheckingName(false);
      return;
    }
    let cancelled = false;
    setCheckingName(true);
    const handle = window.setTimeout(() => {
      void checkHostedCommunityName(normalizedName)
        .then((response) => {
          if (!cancelled)
            setAvailability(
              response.error ? null : (response.available ?? false),
            );
        })
        .catch(() => {
          if (!cancelled) setAvailability(null);
        })
        .finally(() => {
          if (!cancelled) setCheckingName(false);
        });
    }, 500);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [normalizedName, ready, validName]);

  const create = (event: React.FormEvent) => {
    event.preventDefault();
    if (!validName || !identity || identityMismatch || atCommunityLimit) return;
    void run("Creating community…", async () => {
      const available = await checkHostedCommunityName(normalizedName);
      if (available.error || !available.available) {
        setAvailability(false);
        throw new Error(
          hostedCommunityErrorMessage(
            available.error,
            available.correlation_id,
            "That Buzz address is already taken.",
          ),
        );
      }
      const response = await createHostedCommunity(normalizedName);
      if (response.error || !response.community) {
        throw new Error(
          hostedCommunityErrorMessage(
            response.error,
            response.correlation_id,
            "Could not create the community.",
          ),
        );
      }
      const relayUrl = hostedCommunityRelayUrl(response.community);
      if (!relayUrl) {
        throw new Error(
          "The community was created, but Builderlab did not return its community URL. Try connecting it again from settings.",
        );
      }
      const started = onboarding.start({
        source: "add-community",
        relayUrl,
        communityName: response.community.name ?? response.community.slug,
      });
      if (!started) {
        throw new Error(
          "Finish connecting the community already in progress, then try again.",
        );
      }
      onComplete();
    });
  };

  const errorBox = error ? (
    <div
      className="flex items-start gap-3 rounded-xl border border-destructive/30 bg-destructive/10 p-4"
      role="alert"
    >
      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
      <p className="text-sm leading-5 text-destructive">{error}</p>
    </div>
  ) : null;

  if (loading) {
    return (
      <div className="flex min-h-40 items-center justify-center" role="status">
        <LoaderCircle className="h-5 w-5 animate-spin text-muted-foreground" />
        <span className="sr-only">Checking sign-in</span>
      </div>
    );
  }

  if (!auth) {
    return (
      <div className="space-y-5">
        <p className="text-sm leading-6 text-muted-foreground">
          Sign in with Builderlab to create and host a community. Buzz will open
          your browser, then bring you back here.
        </p>
        {errorBox}
        <div className="flex justify-end pt-1">
          <Button disabled={Boolean(action)} onClick={signIn} type="button">
            {action === "Signing in…" ? (
              <LoaderCircle className="h-4 w-4 animate-spin" />
            ) : null}
            {action ?? "Continue to Builderlab"}
            {action ? null : <ExternalLink className="h-4 w-4" />}
          </Button>
        </div>
      </div>
    );
  }

  if (!identity) {
    return (
      <div className="space-y-5">
        <p className="text-sm leading-6 text-muted-foreground">
          Connect this device’s Buzz identity to your Builderlab account. Your
          private key stays on this device.
        </p>
        {errorBox}
        <div className="flex justify-end gap-2 pt-1">
          <Button
            disabled={Boolean(action)}
            onClick={() => void signOut()}
            type="button"
            variant="outline"
          >
            Use a different account
          </Button>
          <Button
            disabled={Boolean(action)}
            onClick={() => void connectIdentity()}
            type="button"
          >
            {action ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
            {action ?? "Connect and continue"}
          </Button>
        </div>
      </div>
    );
  }

  if (identityMismatch) {
    return (
      <div className="space-y-5">
        <p className="text-sm leading-6 text-muted-foreground">
          This Builderlab account uses a different Buzz identity. Switch it to
          this device, or sign in with another account.
        </p>
        <div className="rounded-xl bg-muted/40 px-4 py-3 font-mono text-xs text-muted-foreground">
          <p className="break-all">Account: {identity.npub ?? boundPubkey}</p>
          <p className="mt-1 break-all">
            This device: {localNpub ?? localPubkey}
          </p>
        </div>
        {errorBox}
        <div className="flex justify-end gap-2 pt-1">
          <Button
            disabled={Boolean(action)}
            onClick={() => void signOut()}
            type="button"
            variant="outline"
          >
            Use a different account
          </Button>
          <Button
            disabled={Boolean(action)}
            onClick={() => void switchToDeviceIdentity()}
            type="button"
          >
            {action ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
            {action ?? "Use this device"}
          </Button>
        </div>
      </div>
    );
  }

  const feedback = atCommunityLimit
    ? `You’ve reached the limit of ${HOSTED_COMMUNITY_LIMIT} hosted communities.`
    : name && !validName
      ? "Use lowercase letters, numbers, and single hyphens."
      : checkingName
        ? "Checking availability…"
        : availability === false
          ? "That address is already taken."
          : availability === true
            ? "That address is available."
            : "You can’t change this address after creating the community.";

  return (
    <form className="space-y-5" onSubmit={create}>
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="hosted-community-create-name"
        >
          Community address
        </label>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            CHANNEL_FORM_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            autoFocus
            className={cn(
              "h-8 min-w-0 px-0 py-0 leading-6",
              CHANNEL_FORM_FIELD_CONTROL_CLASS,
            )}
            data-testid="hosted-community-create-name"
            disabled={Boolean(action) || atCommunityLimit}
            id="hosted-community-create-name"
            maxLength={63}
            onChange={(event) => {
              setName(event.target.value.toLowerCase());
              setAvailability(null);
              setError(null);
            }}
            placeholder="north-star"
            spellCheck={false}
            value={name}
          />
          <span className="shrink-0 text-sm text-muted-foreground/70">
            .{HOSTED_COMMUNITY_SUFFIX}
          </span>
        </div>
        <p
          className={cn(
            "text-xs leading-5",
            availability === false || (name && !validName)
              ? "text-destructive"
              : "text-muted-foreground",
          )}
        >
          {feedback}
        </p>
      </div>
      {errorBox}
      <div className="flex justify-end pt-1">
        <Button
          data-testid="hosted-community-create-submit"
          disabled={
            !validName ||
            availability === false ||
            checkingName ||
            Boolean(action) ||
            atCommunityLimit
          }
          type="submit"
        >
          {action ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
          {action ?? "Create community"}
        </Button>
      </div>
    </form>
  );
}
