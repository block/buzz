import * as React from "react";
import { AlertCircle, CheckCircle2, LoaderCircle } from "lucide-react";

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
import { useIdentityQuery } from "@/shared/api/hooks";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { OnboardingFooter } from "@/features/onboarding/ui/OnboardingFooter";

export function HostedCommunityOnboarding({ onBack }: { onBack: () => void }) {
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
        if (loginAttempt.current === attempt) setAction(null);
      });
  };

  const cancelSignIn = () => {
    loginAttempt.current += 1;
    setAction(null);
    setError(null);
    void cancelBuilderlabLogin().catch((cause) => {
      setError(cause instanceof Error ? cause.message : String(cause));
    });
  };

  const signOut = () =>
    run("Signing out…", async () => {
      await clearBuilderlabAuth();
      setAuth(null);
      setIdentity(null);
      setCommunities([]);
      setName("");
      setAvailability(null);
    });

  const goBack = () => {
    void run("Signing out…", async () => {
      await clearBuilderlabAuth();
      onBack();
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
            ? "This device's Buzz identity belongs to a different Builderlab account and can't be moved from here. Sign out, then sign in with the account that already owns this identity."
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

  const activeCommunities = communities.filter(
    (community) => !community.archived_at && hostedCommunityRelayUrl(community),
  );
  const normalizedName = name.trim().toLowerCase();
  const validName =
    normalizedName.length <= 63 &&
    VALID_HOSTED_COMMUNITY_NAME.test(normalizedName);
  const atCommunityLimit = communities.length >= HOSTED_COMMUNITY_LIMIT;

  React.useEffect(() => {
    if (!identity || identityMismatch || !normalizedName || !validName) {
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
  }, [identity, identityMismatch, normalizedName, validName]);

  const connect = (community: HostedCommunity, created = false) => {
    const relayUrl = hostedCommunityRelayUrl(community);
    const retryPrefix = created ? "The community was created, but " : "";
    if (!relayUrl) {
      throw new Error(
        `${retryPrefix}Builderlab did not return its relay address. Try connecting it again, or contact support if it does not appear in your communities.`,
      );
    }
    if (
      !onboarding.start({
        source: "first-community",
        relayUrl,
        communityName: community.name ?? community.slug,
      })
    ) {
      throw new Error(
        `${retryPrefix}onboarding is already in progress for another community. Go back and finish or restart that connection, then connect this community from your owned communities list.`,
      );
    }
  };

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
      connect(response.community, true);
    });
  };

  const busy = action !== null;

  return (
    <div className="flex w-full max-w-[640px] flex-col items-center text-center">
      <h1 className="text-title font-normal">Your communities</h1>
      <p className="mx-auto mt-3 max-w-[480px] text-sm leading-6 text-foreground/80">
        Sign in to connect a community you already own on this machine, or
        create a new one.
      </p>

      <div className="mt-10 w-full space-y-5 text-left">
        {error ? (
          <div
            className="flex items-start gap-2 rounded-xl border border-destructive/40 bg-destructive/5 p-4 text-sm text-destructive"
            role="alert"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{error}</span>
          </div>
        ) : null}

        {loading ? (
          <div className="flex justify-center py-10" role="status">
            <LoaderCircle className="h-6 w-6 animate-spin" />
            <span className="sr-only">Checking Builderlab sign-in</span>
          </div>
        ) : !auth ? (
          <section className="rounded-xl bg-white/75 p-6 text-center">
            <h2 className="font-medium">Sign in or create an account</h2>
            <p className="mt-2 text-sm text-foreground/70">
              Builderlab provides Block-hosted Buzz communities. Authentication
              opens in your browser and returns here.
            </p>
            {action === "Signing in…" ? (
              <div className="mt-5 flex flex-col items-center gap-3">
                <div className="flex items-center gap-2 text-sm text-foreground/70">
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                  Waiting for your browser…
                </div>
                <Button onClick={cancelSignIn} variant="outline">
                  Cancel sign-in
                </Button>
              </div>
            ) : (
              <Button className="mt-5 rounded-full px-6" onClick={signIn}>
                Continue with Builderlab
              </Button>
            )}
          </section>
        ) : !identity ? (
          <section className="rounded-xl bg-white/75 p-6 text-center">
            <h2 className="font-medium">Connect this Buzz identity</h2>
            <p className="mt-2 text-sm text-foreground/70">
              Link this device’s Buzz key to{" "}
              {auth.email ?? auth.name ?? "your Builderlab account"}. Buzz signs
              a one-time challenge locally; your private key never leaves
              Desktop.
            </p>
            <Button
              className="mt-5 rounded-full px-6"
              disabled={busy}
              onClick={() => void connectIdentity()}
            >
              {busy ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
              {action ?? "Connect this Buzz identity"}
            </Button>
          </section>
        ) : identityMismatch ? (
          <section className="rounded-xl border border-amber-500/50 bg-amber-500/5 p-6">
            <div className="flex items-start gap-2">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
              <div>
                <h2 className="font-medium">
                  This account uses a different Buzz identity
                </h2>
                <p className="mt-2 text-sm text-foreground/70">
                  This Builderlab account is connected to another Buzz identity.
                  You can disconnect that identity and reconnect this device, or
                  sign out to use a different email.
                </p>
                <p className="mt-3 break-all font-mono text-xs text-foreground/60">
                  Account: {identity.npub ?? boundPubkey}
                  <br />
                  This device: {localNpub ?? localPubkey}
                </p>
                <div className="mt-4 flex flex-wrap gap-2">
                  <Button
                    disabled={busy}
                    onClick={() => void switchToDeviceIdentity()}
                  >
                    {action ?? "Use this device's identity"}
                  </Button>
                  <Button
                    disabled={busy}
                    onClick={() => void signOut()}
                    variant="outline"
                  >
                    Sign in with a different email
                  </Button>
                </div>
              </div>
            </div>
          </section>
        ) : (
          <>
            <div className="flex items-center gap-2 rounded-xl bg-white/60 px-4 py-3 text-sm text-foreground/70">
              <CheckCircle2 className="h-4 w-4 text-emerald-600" />
              Signed in{auth.email ? ` as ${auth.email}` : ""} with this Buzz
              identity
            </div>

            {activeCommunities.length > 0 ? (
              <section className="rounded-xl bg-white/75 p-5">
                <h2 className="font-medium">Connect to one you own</h2>
                <ul className="mt-3 space-y-2">
                  {activeCommunities.map((community, index) => (
                    <li
                      className="flex items-center justify-between gap-4 rounded-lg border border-foreground/10 bg-white/50 p-3"
                      key={community.id ?? community.normalized_host ?? index}
                    >
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium">
                          {community.name ??
                            community.slug ??
                            "Hosted community"}
                        </p>
                        <p className="truncate text-xs text-foreground/60">
                          {community.normalized_host}
                        </p>
                      </div>
                      <Button
                        className="rounded-full"
                        disabled={busy}
                        onClick={() =>
                          void run("Connecting community…", async () => {
                            connect(community);
                          })
                        }
                        size="sm"
                        variant="outline"
                      >
                        Connect
                      </Button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}

            <form className="rounded-xl bg-white/75 p-5" onSubmit={create}>
              <h2 className="font-medium">Create a new community</h2>
              <p className="mt-1 text-sm text-foreground/70">
                Choose the address your team will use to connect.
              </p>
              <div className="mt-4 flex items-center gap-2">
                <Input
                  aria-label="Community address"
                  autoComplete="off"
                  disabled={busy || atCommunityLimit}
                  maxLength={63}
                  onChange={(event) => {
                    setName(event.target.value.toLowerCase());
                    setAvailability(null);
                  }}
                  placeholder="north-star"
                  spellCheck={false}
                  value={name}
                />
                <span className="shrink-0 text-sm text-foreground/60">
                  .{HOSTED_COMMUNITY_SUFFIX}
                </span>
              </div>
              {atCommunityLimit ? (
                <p className="mt-2 text-sm text-foreground/60">
                  You’ve reached the limit of {HOSTED_COMMUNITY_LIMIT} hosted
                  communities.
                </p>
              ) : name && !validName ? (
                <p className="mt-2 text-sm text-destructive">
                  Use lowercase letters, numbers, and single hyphens.
                </p>
              ) : checkingName ? (
                <p className="mt-2 text-sm text-foreground/60">
                  Checking availability…
                </p>
              ) : availability === false ? (
                <p className="mt-2 text-sm text-destructive">
                  That address is already taken.
                </p>
              ) : availability === true ? (
                <p className="mt-2 text-sm text-emerald-600">
                  That address is available.
                </p>
              ) : null}
              <Button
                className="mt-4 rounded-full px-6"
                disabled={
                  !validName ||
                  availability === false ||
                  checkingName ||
                  busy ||
                  atCommunityLimit
                }
                type="submit"
              >
                {busy ? (
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                ) : null}
                {action ?? "Create and connect"}
              </Button>
            </form>
          </>
        )}
      </div>

      <OnboardingFooter>
        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
          disabled={busy}
          onClick={goBack}
          type="button"
          variant="ghost"
        >
          Back
        </Button>
      </OnboardingFooter>
    </div>
  );
}
