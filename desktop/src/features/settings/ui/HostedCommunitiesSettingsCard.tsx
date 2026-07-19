import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertCircle,
  CheckCircle2,
  ExternalLink,
  LoaderCircle,
  LogOut,
  RefreshCw,
} from "lucide-react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

const HOST_SUFFIX = "communities.buzz.xyz";
const VALID_NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

type BuilderlabAuth = {
  email?: string;
  name?: string;
  expiresAt: string;
};

type ApiError = {
  code?: string;
  message?: string;
  setup_needed?: boolean;
};

type NostrIdentity = {
  npub?: string;
  pubkey_hex?: string;
};

type IdentityResponse = {
  identity?: NostrIdentity;
  error?: ApiError;
  correlation_id?: string;
};

type HostedCommunity = {
  id?: string;
  name?: string;
  slug?: string;
  normalized_host?: string;
  archived_at?: string | null;
};

type CommunitiesResponse = {
  communities?: HostedCommunity[];
  error?: ApiError;
  correlation_id?: string;
};

type AvailabilityResponse = {
  available?: boolean;
  normalized_host?: string;
  error?: ApiError;
  correlation_id?: string;
};

type CreateResponse = {
  community?: HostedCommunity;
  error?: ApiError;
  correlation_id?: string;
};

function errorMessage(
  error: ApiError | undefined,
  correlationId: string | undefined,
  fallback: string,
) {
  const messages: Record<string, string> = {
    missing_mapping: "Connect your Buzz identity before creating a community.",
    invalid_name: "Use lowercase letters, numbers, and hyphens.",
    taken: "That Buzz address is already taken.",
    limit_reached: "You have reached the hosted community limit.",
    relay_unavailable: "Community provisioning is temporarily unavailable.",
    identity_already_bound:
      "This Builderlab account is connected to another Buzz identity.",
    pubkey_already_bound:
      "This Buzz identity is connected to another Builderlab account.",
  };
  const message = messages[error?.code ?? ""] ?? error?.message ?? fallback;
  return correlationId
    ? `${message} Correlation ID: ${correlationId}`
    : message;
}

function relayUrl(community: HostedCommunity) {
  const host = community.normalized_host?.trim();
  return host ? `wss://${host.replace(/^wss?:\/\//, "")}` : null;
}

export function HostedCommunitiesSettingsCard() {
  const onboarding = useCommunityOnboarding();
  const [auth, setAuth] = React.useState<BuilderlabAuth | null>(null);
  const [communities, setCommunities] = React.useState<HostedCommunity[]>([]);
  const [identity, setIdentity] = React.useState<NostrIdentity | null>(null);
  const [name, setName] = React.useState("");
  const [availability, setAvailability] = React.useState<boolean | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [action, setAction] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const loadAccount = React.useCallback(async () => {
    setError(null);
    const [identityResponse, communitiesResponse] = await Promise.all([
      invoke<IdentityResponse>("get_builderlab_nostr_identity"),
      invoke<CommunitiesResponse>("list_builderlab_communities"),
    ]);
    if (
      identityResponse.error &&
      identityResponse.error.code !== "unauthorized"
    ) {
      throw new Error(
        errorMessage(
          identityResponse.error,
          identityResponse.correlation_id,
          "Could not load the connected Buzz identity.",
        ),
      );
    }
    if (communitiesResponse.error && !communitiesResponse.error.setup_needed) {
      throw new Error(
        errorMessage(
          communitiesResponse.error,
          communitiesResponse.correlation_id,
          "Could not load communities.",
        ),
      );
    }
    setIdentity(identityResponse.identity ?? null);
    setCommunities(communitiesResponse.communities ?? []);
  }, []);

  React.useEffect(() => {
    let active = true;
    void invoke<BuilderlabAuth | null>("get_builderlab_auth")
      .then(async (nextAuth) => {
        if (!active) return;
        setAuth(nextAuth);
        if (nextAuth) await loadAccount();
      })
      .catch((cause) => {
        if (active) setError(String(cause));
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

  const signIn = () =>
    run("Signing in…", async () => {
      const nextAuth = await invoke<BuilderlabAuth>("start_builderlab_login");
      setAuth(nextAuth);
      await loadAccount();
    });

  const signOut = () =>
    run("Signing out…", async () => {
      await invoke("clear_builderlab_auth");
      setAuth(null);
      setIdentity(null);
      setCommunities([]);
      setName("");
      setAvailability(null);
    });

  const connectIdentity = () =>
    run("Connecting Buzz identity…", async () => {
      const response = await invoke<IdentityResponse>(
        "bind_builderlab_nostr_identity",
      );
      if (response.error) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not connect the Buzz identity.",
          ),
        );
      }
      setIdentity(response.identity ?? null);
      await loadAccount();
    });

  const normalizedName = name.trim().toLowerCase();
  const validName =
    normalizedName.length <= 63 && VALID_NAME.test(normalizedName);

  const checkAvailability = () => {
    if (!validName) return Promise.resolve();
    return run("Checking availability…", async () => {
      const response = await invoke<AvailabilityResponse>(
        "check_builderlab_community_name",
        { name: normalizedName },
      );
      if (response.error) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not check this address.",
          ),
        );
      }
      setAvailability(response.available ?? false);
    });
  };

  const createCommunity = (event: React.FormEvent) => {
    event.preventDefault();
    if (!validName || !identity) return;
    void run("Creating community…", async () => {
      const availabilityResponse = await invoke<AvailabilityResponse>(
        "check_builderlab_community_name",
        { name: normalizedName },
      );
      if (availabilityResponse.error || !availabilityResponse.available) {
        setAvailability(false);
        throw new Error(
          errorMessage(
            availabilityResponse.error,
            availabilityResponse.correlation_id,
            "That Buzz address is already taken.",
          ),
        );
      }
      const response = await invoke<CreateResponse>(
        "create_builderlab_community",
        { name: normalizedName },
      );
      if (response.error || !response.community) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not create the community.",
          ),
        );
      }
      const url = relayUrl(response.community);
      if (!url)
        throw new Error("The new community did not return a relay address.");
      setName("");
      setAvailability(null);
      await loadAccount();
      if (
        !onboarding.start({
          source: "add-community",
          relayUrl: url,
          communityName: response.community.name ?? normalizedName,
        })
      ) {
        throw new Error(
          "Another community is already being connected. Finish it before connecting this one.",
        );
      }
    });
  };

  return (
    <section className="space-y-6" data-testid="hosted-communities-settings">
      <SettingsSectionHeader
        title="Hosted communities"
        description="Create and connect Block-hosted Buzz communities. Builderlab sign-in is only used on this page."
      />

      {error ? (
        <div className="flex items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive">
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>{error}</span>
        </div>
      ) : null}

      {loading ? (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <LoaderCircle className="h-4 w-4 animate-spin" /> Checking sign-in…
        </div>
      ) : !auth ? (
        <div className="rounded-xl border border-border/70 p-5">
          <h3 className="font-medium">Sign in to manage hosted communities</h3>
          <p className="mt-2 max-w-2xl text-sm text-muted-foreground">
            Authentication opens in your browser and returns securely to Buzz.
            You can use every other part of the app without signing in.
          </p>
          <Button
            className="mt-4"
            disabled={action != null}
            onClick={() => void signIn()}
          >
            {action ? (
              <LoaderCircle className="h-4 w-4 animate-spin" />
            ) : (
              <ExternalLink className="h-4 w-4" />
            )}
            {action ?? "Sign in with Builderlab"}
          </Button>
        </div>
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border/70 p-4">
            <div>
              <p className="text-sm font-medium">
                {auth.name || auth.email || "Builderlab account"}
              </p>
              {auth.name && auth.email ? (
                <p className="text-xs text-muted-foreground">{auth.email}</p>
              ) : null}
            </div>
            <Button
              variant="outline"
              size="sm"
              disabled={action != null}
              onClick={() => void signOut()}
            >
              <LogOut className="h-4 w-4" /> Sign out
            </Button>
          </div>

          {!identity ? (
            <div className="rounded-xl border border-amber-500/40 bg-amber-500/5 p-5">
              <h3 className="font-medium">Connect this Buzz identity</h3>
              <p className="mt-2 text-sm text-muted-foreground">
                Community ownership is tied to your current Buzz key. Buzz will
                sign a one-time challenge locally; your private key never leaves
                Desktop.
              </p>
              <Button
                className="mt-4"
                disabled={action != null}
                onClick={() => void connectIdentity()}
              >
                {action ? (
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                ) : null}
                {action ?? "Connect Buzz identity"}
              </Button>
            </div>
          ) : (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <CheckCircle2 className="h-4 w-4 text-emerald-500" /> Buzz
              identity connected
              {identity.npub ? (
                <span className="font-mono text-xs">{identity.npub}</span>
              ) : null}
            </div>
          )}

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className="font-medium">Your communities</h3>
              <Button
                variant="ghost"
                size="sm"
                disabled={action != null}
                onClick={() => void run("Refreshing…", loadAccount)}
              >
                <RefreshCw className="h-4 w-4" /> Refresh
              </Button>
            </div>
            {communities.length === 0 ? (
              <p className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">
                No hosted communities yet.
              </p>
            ) : (
              <ul className="space-y-2">
                {communities.map((community, index) => {
                  const url = relayUrl(community);
                  return (
                    <li
                      className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border/70 p-4"
                      key={community.id ?? community.normalized_host ?? index}
                    >
                      <div>
                        <p className="text-sm font-medium">
                          {community.name ??
                            community.slug ??
                            "Hosted community"}
                        </p>
                        <p className="text-xs text-muted-foreground">
                          {community.normalized_host}
                          {community.archived_at ? " · Archived" : ""}
                        </p>
                      </div>
                      {url && !community.archived_at ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() =>
                            onboarding.start({
                              source: "add-community",
                              relayUrl: url,
                              communityName: community.name,
                            })
                          }
                        >
                          Connect
                        </Button>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          <form
            className="space-y-4 rounded-xl border border-border/70 p-5"
            onSubmit={createCommunity}
          >
            <div>
              <h3 className="font-medium">Create a community</h3>
              <p className="mt-1 text-sm text-muted-foreground">
                Choose the address your team will use to connect.
              </p>
            </div>
            <div className="flex max-w-xl items-center gap-2">
              <Input
                aria-label="Community address"
                autoComplete="off"
                disabled={!identity || action != null}
                maxLength={63}
                onBlur={() => void checkAvailability()}
                onChange={(event) => {
                  setName(event.target.value.toLowerCase());
                  setAvailability(null);
                }}
                placeholder="north-star"
                spellCheck={false}
                value={name}
              />
              <span className="shrink-0 text-sm text-muted-foreground">
                .{HOST_SUFFIX}
              </span>
            </div>
            {name && !validName ? (
              <p className="text-sm text-destructive">
                Use lowercase letters, numbers, and single hyphens.
              </p>
            ) : null}
            {availability === false ? (
              <p className="text-sm text-destructive">
                That address is already taken.
              </p>
            ) : null}
            {availability === true ? (
              <p className="text-sm text-emerald-600">
                That address is available.
              </p>
            ) : null}
            <Button
              disabled={
                !identity ||
                !validName ||
                availability === false ||
                action != null
              }
              type="submit"
            >
              {action ? (
                <LoaderCircle className="h-4 w-4 animate-spin" />
              ) : null}
              {action ?? "Create and connect"}
            </Button>
          </form>
        </>
      )}
    </section>
  );
}
