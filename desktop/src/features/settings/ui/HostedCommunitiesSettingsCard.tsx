import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertCircle,
  Archive,
  ArchiveRestore,
  ArrowLeftRight,
  CheckCircle2,
  ExternalLink,
  LoaderCircle,
  LogOut,
  RefreshCw,
  Unlink,
} from "lucide-react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button, buttonVariants } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
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
  owner_pubkey?: string;
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

type CommunityMutationResponse = {
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
    not_owner: "Only the community owner can do that.",
    transferee_not_registered:
      "That person needs a connected Buzz identity before you can transfer ownership to them.",
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
  const localPubkey = useIdentityQuery().data?.pubkey ?? null;
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

  // Returns whether the operation completed without throwing so callers (e.g.
  // dialogs) can close themselves only on success.
  const run = async (label: string, operation: () => Promise<void>) => {
    setAction(label);
    setError(null);
    try {
      await operation();
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return false;
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

  const unpairIdentity = () =>
    run("Unpairing identity…", async () => {
      const response = await invoke<IdentityResponse>(
        "delete_builderlab_nostr_identity",
      );
      if (response.error) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not unpair the Buzz identity.",
          ),
        );
      }
      setIdentity(null);
      await loadAccount();
    });

  // The Builderlab account can be bound to an npub that differs from the key
  // this Desktop is currently signing with (e.g. you signed into an email tied
  // to a different test identity). When that happens the community list and
  // Connect buttons operate on the *bound* npub's communities, so "Connect"
  // would drop you into a relay your local key isn't a member of. Detect it and
  // block Connect + Create until the identities match.
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
      // The account is bound to a different npub, so re-binding directly returns
      // identity_already_bound. Release the current binding first, then bind
      // this device's key. If the local key is reserved by another Builderlab
      // account, the bind fails with pubkey_already_bound — surface that instead
      // of leaving the swap half-finished silently.
      const released = await invoke<IdentityResponse>(
        "delete_builderlab_nostr_identity",
      );
      if (released.error) {
        throw new Error(
          errorMessage(
            released.error,
            released.correlation_id,
            "Could not release the previously connected Buzz identity.",
          ),
        );
      }
      const bound = await invoke<IdentityResponse>(
        "bind_builderlab_nostr_identity",
      );
      if (bound.error) {
        // Refresh so the UI reflects the now-unbound account before surfacing
        // the reason the swap could not complete.
        await loadAccount();
        throw new Error(
          bound.error.code === "pubkey_already_bound"
            ? "This device's Buzz identity is already reserved by another Builderlab account, so it can't be connected here. Sign in with that account, or transfer the identity there first."
            : errorMessage(
                bound.error,
                bound.correlation_id,
                "Could not connect this device's Buzz identity.",
              ),
        );
      }
      setIdentity(bound.identity ?? null);
      await loadAccount();
    });

  const archiveCommunity = (community: HostedCommunity) => {
    if (!community.id) return Promise.resolve(false);
    return run("Archiving community…", async () => {
      const response = await invoke<CommunityMutationResponse>(
        "archive_builderlab_community",
        { communityId: community.id },
      );
      // Treat a returned archived timestamp as success even if the payload also
      // carries a soft error (existing connections may take time to close).
      if (response.error && !response.community?.archived_at) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not archive the community.",
          ),
        );
      }
      await loadAccount();
    });
  };

  const unarchiveCommunity = (community: HostedCommunity) => {
    if (!community.id) return Promise.resolve(false);
    return run("Unarchiving community…", async () => {
      const response = await invoke<CommunityMutationResponse>(
        "unarchive_builderlab_community",
        { communityId: community.id },
      );
      if (response.error && response.community?.archived_at !== null) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not unarchive the community.",
          ),
        );
      }
      await loadAccount();
    });
  };

  const transferCommunity = (community: HostedCommunity, npub: string) =>
    run("Transferring ownership…", async () => {
      const response = await invoke<CommunityMutationResponse>(
        "transfer_builderlab_community",
        { communityId: community.id, transfereeNpub: npub },
      );
      if (response.error) {
        throw new Error(
          errorMessage(
            response.error,
            response.correlation_id,
            "Could not transfer ownership.",
          ),
        );
      }
      await loadAccount();
    });

  const normalizedName = name.trim().toLowerCase();
  const validName =
    normalizedName.length <= 63 && VALID_NAME.test(normalizedName);

  const checkAvailability = () => {
    if (!validName) return Promise.resolve(false);
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
    if (!validName || !identity || identityMismatch) return;
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
      const response = await invoke<CommunityMutationResponse>(
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

  const busy = action != null;

  return (
    <section className="space-y-6" data-testid="hosted-communities-settings">
      <SettingsSectionHeader
        title="Hosted communities"
        description="Buzz works with any relay. This page is only for relay hosting provided by Block — sign in with a Builderlab account to create and manage Block-hosted communities. Builderlab sign-in is used on this page alone."
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
            disabled={busy}
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
              disabled={busy}
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
                disabled={busy}
                onClick={() => void connectIdentity()}
              >
                {action ? (
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                ) : null}
                {action ?? "Connect Buzz identity"}
              </Button>
            </div>
          ) : identityMismatch ? (
            <div className="rounded-xl border border-amber-500/50 bg-amber-500/5 p-5">
              <div className="flex items-start gap-2">
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
                <div>
                  <h3 className="font-medium">
                    This account is connected to a different Buzz identity
                  </h3>
                  <p className="mt-2 text-sm text-muted-foreground">
                    Your Builderlab account owns communities under another Buzz
                    key, so connecting them here would join a relay this device
                    isn&apos;t a member of. Creating and connecting are paused
                    until the identities match.
                  </p>
                  <dl className="mt-3 space-y-1 text-xs">
                    <div className="flex flex-wrap gap-x-2">
                      <dt className="text-muted-foreground">Account uses</dt>
                      <dd className="font-mono">
                        {identity.npub ?? boundPubkey}
                      </dd>
                    </div>
                    <div className="flex flex-wrap gap-x-2">
                      <dt className="text-muted-foreground">This device</dt>
                      <dd className="font-mono">{localNpub ?? localPubkey}</dd>
                    </div>
                  </dl>
                </div>
              </div>
              <Button
                className="mt-4"
                disabled={busy}
                onClick={() => void switchToDeviceIdentity()}
              >
                {action ? (
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                ) : null}
                {action ?? "Switch to this device's identity"}
              </Button>
            </div>
          ) : (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border/70 p-4">
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <CheckCircle2 className="h-4 w-4 text-emerald-500" /> Buzz
                identity connected
                {identity.npub ? (
                  <span className="font-mono text-xs">{identity.npub}</span>
                ) : null}
              </div>
              <UnpairIdentityButton
                busy={busy}
                onConfirm={() => void unpairIdentity()}
              />
            </div>
          )}

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className="font-medium">Your communities</h3>
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
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
                {[...communities]
                  .sort(
                    (a, b) =>
                      Number(Boolean(a.archived_at)) -
                      Number(Boolean(b.archived_at)),
                  )
                  .map((community, index) => (
                    <CommunityRow
                      key={community.id ?? community.normalized_host ?? index}
                      community={community}
                      busy={busy}
                      canConnect={!identityMismatch}
                      onConnect={() => {
                        const url = relayUrl(community);
                        if (url)
                          onboarding.start({
                            source: "add-community",
                            relayUrl: url,
                            communityName: community.name,
                          });
                      }}
                      onArchive={() => void archiveCommunity(community)}
                      onUnarchive={() => void unarchiveCommunity(community)}
                      onTransfer={(npub) => transferCommunity(community, npub)}
                    />
                  ))}
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
                disabled={!identity || identityMismatch || busy}
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
                identityMismatch ||
                !validName ||
                availability === false ||
                busy
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

function UnpairIdentityButton({
  busy,
  onConfirm,
}: {
  busy: boolean;
  onConfirm: () => void;
}) {
  const [open, setOpen] = React.useState(false);
  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <Button
        variant="ghost"
        size="sm"
        className="text-destructive hover:text-destructive"
        disabled={busy}
        onClick={() => setOpen(true)}
      >
        <Unlink className="h-4 w-4" /> Unpair identity
      </Button>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Unpair this Buzz identity?</AlertDialogTitle>
          <AlertDialogDescription>
            Your Builderlab account will no longer be connected to this Buzz
            key. You can reconnect any key later, but community actions stay
            unavailable until you do.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: "destructive" })}
            onClick={onConfirm}
          >
            Unpair identity
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function CommunityRow({
  community,
  busy,
  canConnect,
  onConnect,
  onArchive,
  onUnarchive,
  onTransfer,
}: {
  community: HostedCommunity;
  busy: boolean;
  canConnect: boolean;
  onConnect: () => void;
  onArchive: () => void;
  onUnarchive: () => void;
  onTransfer: (npub: string) => Promise<boolean>;
}) {
  const [confirmArchive, setConfirmArchive] = React.useState(false);
  const [confirmUnarchive, setConfirmUnarchive] = React.useState(false);
  const [transferOpen, setTransferOpen] = React.useState(false);
  const url = relayUrl(community);
  const archived = Boolean(community.archived_at);
  const displayName = community.name ?? community.slug ?? "Hosted community";

  return (
    <li
      className={`flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border/70 p-4 ${
        archived ? "opacity-70" : ""
      }`}
    >
      <div className="min-w-0">
        <p className="text-sm font-medium">{displayName}</p>
        <p className="text-xs text-muted-foreground">
          {community.normalized_host}
          {archived ? " · Archived" : ""}
        </p>
      </div>

      {archived ? (
        <>
          <Button
            variant="outline"
            size="sm"
            disabled={busy || !community.id}
            onClick={() => setConfirmUnarchive(true)}
          >
            <ArchiveRestore className="h-4 w-4" /> Unarchive
          </Button>
          <AlertDialog
            open={confirmUnarchive}
            onOpenChange={setConfirmUnarchive}
          >
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Unarchive {displayName}?</AlertDialogTitle>
                <AlertDialogDescription>
                  This address becomes connectable again. Connections that
                  closed during archival will not reconnect automatically.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction onClick={onUnarchive}>
                  Unarchive
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          {url && canConnect ? (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={onConnect}
            >
              Connect
            </Button>
          ) : null}
          <Button
            variant="ghost"
            size="sm"
            disabled={busy || !community.id}
            onClick={() => setTransferOpen(true)}
          >
            <ArrowLeftRight className="h-4 w-4" /> Transfer
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive hover:text-destructive"
            disabled={busy || !community.id}
            onClick={() => setConfirmArchive(true)}
          >
            <Archive className="h-4 w-4" /> Archive
          </Button>

          <AlertDialog open={confirmArchive} onOpenChange={setConfirmArchive}>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Archive {displayName}?</AlertDialogTitle>
                <AlertDialogDescription>
                  New and existing connections stop and the address stays
                  reserved. Archiving can&apos;t be undone from here without
                  unarchiving, and the community keeps counting toward your
                  quota — it isn&apos;t deleted.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction
                  className={buttonVariants({ variant: "destructive" })}
                  onClick={onArchive}
                >
                  Archive
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>

          <TransferOwnershipDialog
            open={transferOpen}
            onOpenChange={setTransferOpen}
            communityName={displayName}
            busy={busy}
            onTransfer={onTransfer}
          />
        </div>
      )}
    </li>
  );
}

function TransferOwnershipDialog({
  open,
  onOpenChange,
  communityName,
  busy,
  onTransfer,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  communityName: string;
  busy: boolean;
  onTransfer: (npub: string) => Promise<boolean>;
}) {
  const [npub, setNpub] = React.useState("");
  const npubIsValid = npub.startsWith("npub1") && npub.length >= 50;

  React.useEffect(() => {
    if (!open) setNpub("");
  }, [open]);

  const submit = async () => {
    if (!npubIsValid) return;
    const ok = await onTransfer(npub.trim());
    if (ok) onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Transfer ownership</DialogTitle>
          <DialogDescription>
            Transfer {communityName} to another person. You become a regular
            member. The recipient needs a connected Buzz identity first, and
            this can&apos;t be undone.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Input
            aria-label="Recipient npub"
            autoComplete="off"
            className="font-mono text-sm"
            placeholder="npub1…"
            spellCheck={false}
            value={npub}
            onChange={(event) => setNpub(event.target.value.trim())}
          />
          {npub.length > 0 && !npubIsValid ? (
            <p className="text-sm text-destructive">
              Enter a valid npub that starts with npub1.
            </p>
          ) : null}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={!npubIsValid || busy}
            onClick={() => void submit()}
          >
            {busy ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
            Transfer ownership
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
