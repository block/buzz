import * as React from "react";
import { AlertCircle, LoaderCircle } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

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
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { OnboardingFooter } from "@/features/onboarding/ui/OnboardingFooter";
import {
  ONBOARDING_INK_ICON_CLASS,
  ONBOARDING_PRIMARY_CTA_CLASS,
} from "@/features/onboarding/ui/OnboardingChrome";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";

const FUZZY_SURFACE_CLASS =
  "relative left-1/2 w-[min(calc(100%+12rem),calc(100vw-2rem))] max-w-[1040px] -translate-x-1/2 px-20 pb-14 pt-20 !text-[rgb(var(--buzz-hosted-community-surface-fg))] [--buzz-card-textured-min-height:224px]";
const COMMUNITY_LIST_CLASS = "mx-auto w-full max-w-[520px] text-left";
const COMMUNITY_ROW_CLASS =
  "flex min-h-[5.75rem] items-center justify-between gap-8 py-4 text-sm";
const COMMUNITY_DIVIDER_CLASS =
  "border-b-[0.5px] border-[rgb(var(--buzz-hosted-community-divider-border)/0.5)]";
const COMMUNITY_ACTION_CLASS =
  "h-[2.375rem] min-w-32 shrink-0 rounded-full bg-[rgb(var(--buzz-hosted-community-action-bg))] px-6 text-sm text-foreground shadow-none hover:bg-[rgb(var(--buzz-hosted-community-action-bg-hover))]";
const PAGE_CTA_CLASS = `${ONBOARDING_PRIMARY_CTA_CLASS} w-36 shadow-none`;
const PAGE_BACK_CLASS =
  "h-[2.375rem] w-36 rounded-full bg-foreground/10 px-6 shadow-none hover:bg-foreground/15";
const MODAL_PRIMARY_ACTION_CLASS = `${ONBOARDING_PRIMARY_CTA_CLASS} !text-[rgb(var(--buzz-hosted-community-modal-action-fg))]`;
const MODAL_BACK_ACTION_CLASS =
  "h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15";

type HostedCommunityOnboardingProps = {
  onBack: () => void;
  /**
   * Fires once the account is signed in with a linked identity — the parent
   * uses this to reveal the stage page only after the sign-in modal has
   * finished driving the flow.
   */
  onReady?: () => void;
  /**
   * While true, render only the sign-in modal and keep the page scaffolding
   * hidden, so whatever screen launched the flow stays visible behind it.
   */
  stageHidden?: boolean;
};

export function HostedCommunityOnboarding({
  onBack,
  onReady,
  stageHidden = false,
}: HostedCommunityOnboardingProps) {
  const onboarding = useCommunityOnboarding();
  const shouldReduceMotion = useReducedMotion();
  const localPubkey = useIdentityQuery().data?.pubkey ?? null;
  const [auth, setAuth] = React.useState<BuilderlabAuth | null>(null);
  const [identity, setIdentity] = React.useState<HostedNostrIdentity | null>(
    null,
  );
  const [communities, setCommunities] = React.useState<HostedCommunity[]>([]);
  const [showCreate, setShowCreate] = React.useState(false);
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

  const cancelSignInAndGoBack = () => {
    loginAttempt.current += 1;
    setAction(null);
    setError(null);
    onBack();
    void cancelBuilderlabLogin().catch(() => {
      // The modal has already closed and the login attempt is invalidated;
      // cancellation is best-effort cleanup for the native/browser flow.
    });
  };

  const signOut = () =>
    run("Signing out…", async () => {
      await clearBuilderlabAuth();
      setAuth(null);
      setIdentity(null);
      setCommunities([]);
      setShowCreate(false);
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
  const hasCommunities = activeCommunities.length > 0;

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
        firstCommunityPage: "owned",
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
  // The account is set up once we're signed in with a linked, matching
  // identity. Until then the sign-in / link-identity modal drives the flow and
  // the page behind it shows a blurred preview of where communities will land.
  const ready = Boolean(auth && identity && !identityMismatch);
  const modalOpen = !loading && !ready;

  // Tell the parent once sign-in completes so it can reveal the stage page.
  // Guarded so it fires exactly once per mount.
  const readyNotifiedRef = React.useRef(false);
  React.useEffect(() => {
    if (loading || !ready || readyNotifiedRef.current) return;
    readyNotifiedRef.current = true;
    onReady?.();
  }, [loading, onReady, ready]);

  const errorBox = error ? (
    <div
      className="flex items-start gap-3 rounded-xl border border-destructive/30 bg-destructive/10 p-4 text-left"
      role="alert"
    >
      <span className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-destructive/15 text-destructive">
        <AlertCircle className="h-4 w-4" />
      </span>
      <span className="text-sm font-medium leading-5 text-destructive">
        {error}
      </span>
    </div>
  ) : null;

  const creationFeedback = atCommunityLimit
    ? `You’ve reached the limit of ${HOSTED_COMMUNITY_LIMIT} hosted communities.`
    : name && !validName
      ? "Use lowercase letters, numbers, and single hyphens."
      : checkingName
        ? "Checking availability…"
        : availability === false
          ? "That address is already taken."
          : availability === true
            ? "That address is available."
            : null;

  // The composed `<name>.<suffix>` line renders at text-4xl, but a valid name
  // can be up to 63 chars and the suffix adds another 21 — far wider than the
  // card at the 800px app minimum. Scale the font down to fit the container
  // (container-query width) while capping at text-4xl (2.25rem) so short names
  // keep the full-size treatment and long names stay fully visible instead of
  // overflowing the surface.
  const composedAddressLength =
    (name ? name.length : "your-community".length) +
    HOSTED_COMMUNITY_SUFFIX.length +
    1; // leading dot before the suffix
  // ~0.62em is the monospace glyph advance; 90cqw leaves a safety margin so the
  // glyphs never touch the card edge.
  const addressFontSize = `min(2.25rem, calc(90cqw / ${(
    composedAddressLength * 0.62
  ).toFixed(2)}))`;

  const creationInput = (inline: boolean) => (
    <Input
      aria-describedby={
        creationFeedback ? "hosted-community-feedback" : undefined
      }
      aria-label="Community name"
      autoComplete="off"
      className={
        inline
          ? "h-[2.375rem] w-[16.5rem] rounded-full border border-[color:var(--buzz-onboarding-backup-ink)]/25 bg-[rgb(var(--buzz-hosted-community-input-bg)/0.6)] px-6 text-center text-sm shadow-none placeholder:text-foreground/30 focus-visible:ring-1 focus-visible:ring-[color:var(--buzz-onboarding-backup-ink)]/40"
          : "h-auto min-w-0 flex-none rounded-none border-0 bg-transparent p-0 text-right font-mono !text-[rgb(var(--buzz-hosted-community-surface-fg))] shadow-none placeholder:!text-[rgb(var(--buzz-hosted-community-surface-fg))] placeholder:opacity-20 focus-visible:ring-0"
      }
      disabled={busy || atCommunityLimit}
      id="hosted-community-address"
      data-testid={!inline ? "hosted-community-address-input" : undefined}
      maxLength={63}
      onChange={(event) => {
        setName(event.target.value.toLowerCase());
        setAvailability(null);
      }}
      placeholder={inline ? "Community name here" : "your-community"}
      spellCheck={false}
      style={
        inline
          ? undefined
          : {
              width: `${name ? name.length : "your-community".length}ch`,
              fontSize: addressFontSize,
            }
      }
      value={name}
    />
  );

  const renderCreationForm = (inline: boolean) =>
    inline ? (
      <form
        className="relative"
        id="hosted-community-create-form"
        onSubmit={create}
      >
        <div className={COMMUNITY_ROW_CLASS}>
          <label className="text-sm" htmlFor="hosted-community-address">
            Set new community name
          </label>
          {creationInput(true)}
        </div>
      </form>
    ) : (
      <Card
        asChild
        className={`${FUZZY_SURFACE_CLASS} !py-10 [--buzz-card-textured-min-height:176px]`}
        data-testid="hosted-community-create-surface"
        variant="textured"
      >
        <form id="hosted-community-create-form" onSubmit={create}>
          <div
            className="mx-auto flex w-full max-w-[900px] items-center justify-center whitespace-nowrap"
            data-testid="hosted-community-address-line"
            style={{ containerType: "inline-size" }}
          >
            {creationInput(false)}
            <span
              className="shrink-0 font-mono !text-[rgb(var(--buzz-hosted-community-surface-fg))]"
              id="hosted-community-suffix"
              style={{ fontSize: addressFontSize }}
            >
              .{HOSTED_COMMUNITY_SUFFIX}
            </span>
          </div>
        </form>
      </Card>
    );

  const signInDialog = (
    <Dialog
      open={modalOpen}
      onOpenChange={(open) => {
        if (open) return;
        if (action === "Signing in…") {
          cancelSignInAndGoBack();
          return;
        }
        if (!busy) goBack();
      }}
    >
      <DialogContent
        className="buzz-onboarding-neutral-theme max-w-[560px] text-foreground [&_button]:shadow-none"
        closeButtonClassName={ONBOARDING_INK_ICON_CLASS}
        data-system-color-scheme="light"
        overlayClassName="bg-[rgb(var(--buzz-hosted-community-modal-overlay-bg)/0.25)]"
        surface="textured"
      >
        <div className="mx-auto flex w-full max-w-sm flex-col items-center py-2 text-center">
          <BuzzMark className="mb-5 h-auto w-9 text-foreground" />

          {!auth ? (
            <>
              <DialogTitle className="text-xl font-medium text-foreground">
                Set up your community
              </DialogTitle>
              <DialogDescription className="mt-2 text-sm leading-6 text-foreground">
                Sign in to connect a community you already own or create a new
                one. We’ll open Builderlab in your browser, then bring you back
                to Buzz.
              </DialogDescription>
              {errorBox ? <div className="mt-5 w-full">{errorBox}</div> : null}
              {action === "Signing in…" ? (
                <Button
                  className={`mt-6 ${MODAL_PRIMARY_ACTION_CLASS}`}
                  disabled
                >
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                  Waiting for your browser…
                </Button>
              ) : (
                <Button
                  className={`mt-6 ${MODAL_PRIMARY_ACTION_CLASS}`}
                  onClick={signIn}
                >
                  Sign in to continue
                </Button>
              )}
              {/* Quiet breadcrumb: Buzz itself is open source; this hosted
                    relay is the one account-backed piece of the flow. */}
              <p className="mt-6 w-full border-t border-foreground/10 pt-4 text-xs leading-5 text-foreground/45">
                Buzz is open source. Builderlab hosts the relay for this
                account.
              </p>
            </>
          ) : !identity ? (
            <>
              <DialogTitle className="text-xl font-medium text-foreground">
                Finish connecting Buzz
              </DialogTitle>
              <DialogDescription className="mt-2 text-sm leading-6 text-foreground">
                Your Builderlab account
                {auth.email ? ` (${auth.email})` : ""} is ready. Connect this
                device’s Buzz identity to finish setup. Your private key stays
                on this device.
              </DialogDescription>
              {errorBox ? <div className="mt-5 w-full">{errorBox}</div> : null}
              <Button
                className={`mt-6 ${MODAL_PRIMARY_ACTION_CLASS}`}
                disabled={busy}
                onClick={() => void connectIdentity()}
              >
                {busy ? (
                  <LoaderCircle className="h-4 w-4 animate-spin" />
                ) : null}
                {busy ? action : "Connect and continue"}
              </Button>
            </>
          ) : (
            <>
              <DialogTitle className="text-xl font-medium text-foreground">
                This account uses a different Buzz identity
              </DialogTitle>
              <DialogDescription className="mt-2 text-sm leading-6 text-foreground">
                This account is connected to another Buzz identity. Reconnect
                this device, or sign out to use a different email.
              </DialogDescription>
              <p className="mt-4 w-full break-all rounded-xl bg-[rgb(var(--buzz-hosted-community-identity-bg)/0.5)] px-4 py-3 text-left font-mono text-xs text-foreground">
                Account: {identity.npub ?? boundPubkey}
                <br />
                This device: {localNpub ?? localPubkey}
              </p>
              {errorBox ? <div className="mt-5 w-full">{errorBox}</div> : null}
              <div className="mt-6 flex flex-col items-stretch gap-2">
                <Button
                  className={MODAL_PRIMARY_ACTION_CLASS}
                  disabled={busy}
                  onClick={() => void switchToDeviceIdentity()}
                >
                  {busy ? action : "Use this device's identity"}
                </Button>
                <Button
                  className={MODAL_BACK_ACTION_CLASS}
                  disabled={busy}
                  onClick={() => void signOut()}
                  variant="ghost"
                >
                  Sign in with a different email
                </Button>
              </div>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );

  // While the sign-in modal drives the flow, keep the launching page
  // visible behind it instead of swapping to this stage prematurely.
  if (stageHidden) {
    return signInDialog;
  }

  return (
    <div className="flex min-h-[calc(100dvh-15.625rem)] w-full max-w-[920px] flex-col items-center text-center">
      <h1 className="max-w-[620px] text-title font-normal leading-[1.18] tracking-[-0.025em]">
        {hasCommunities ? "Choose a community" : "Create a community"}
      </h1>
      <p className="mx-auto mt-2 max-w-[560px] text-sm leading-6 text-foreground">
        {hasCommunities
          ? "Connect one you own, or start something new."
          : "Claim a Buzz address to get started."}
      </p>

      <div className="flex w-full flex-1 flex-col justify-center text-left">
        {loading ? (
          <div className="flex justify-center py-10" role="status">
            <LoaderCircle className="h-6 w-6 animate-spin" />
            <span className="sr-only">Checking sign-in</span>
          </div>
        ) : ready ? (
          <>
            {errorBox}
            {hasCommunities ? (
              <>
                <Card
                  className={`${FUZZY_SURFACE_CLASS} !max-w-[760px]`}
                  data-testid="hosted-community-list-surface"
                  variant="textured"
                >
                  <section className={COMMUNITY_LIST_CLASS}>
                    <h2 className="text-center text-sm font-medium">
                      Your communities
                    </h2>
                    <ul className="mt-2">
                      {activeCommunities.map((community, index) => (
                        <li
                          className={`${COMMUNITY_ROW_CLASS} ${COMMUNITY_DIVIDER_CLASS}`}
                          key={
                            community.id ?? community.normalized_host ?? index
                          }
                        >
                          <div className="min-w-0">
                            <p className="truncate text-sm">
                              {community.name ??
                                community.slug ??
                                "Hosted community"}
                            </p>
                            <p className="mt-1 truncate text-sm text-foreground/55">
                              {community.normalized_host}
                            </p>
                          </div>
                          <Button
                            className={COMMUNITY_ACTION_CLASS}
                            disabled={busy}
                            onClick={() =>
                              void run("Connecting community…", async () => {
                                connect(community);
                              })
                            }
                            size="sm"
                            variant="ghost"
                          >
                            Connect
                          </Button>
                        </li>
                      ))}
                    </ul>
                    <AnimatePresence initial={false} mode="wait">
                      {!showCreate ? (
                        <motion.div
                          animate={{ opacity: 1, transform: "translateX(0px)" }}
                          className={COMMUNITY_ROW_CLASS}
                          exit={
                            shouldReduceMotion
                              ? { opacity: 0 }
                              : {
                                  opacity: 0,
                                  transform: "translateX(-12px)",
                                }
                          }
                          initial={false}
                          key="add-community-action"
                          transition={{
                            duration: shouldReduceMotion ? 0 : 0.18,
                            ease: "easeOut",
                          }}
                        >
                          <p className="text-sm">
                            Want to create a new community?
                          </p>
                          <Button
                            className={COMMUNITY_ACTION_CLASS}
                            disabled={busy || atCommunityLimit}
                            onClick={() => setShowCreate(true)}
                            size="sm"
                            type="button"
                            variant="ghost"
                          >
                            + Add new
                          </Button>
                        </motion.div>
                      ) : (
                        <motion.div
                          animate={{ opacity: 1, transform: "translateX(0px)" }}
                          initial={
                            shouldReduceMotion
                              ? { opacity: 1 }
                              : {
                                  opacity: 0,
                                  transform: "translateX(12px)",
                                }
                          }
                          key="add-community-input"
                          transition={{
                            duration: shouldReduceMotion ? 0 : 0.22,
                            ease: "easeOut",
                          }}
                        >
                          {renderCreationForm(true)}
                        </motion.div>
                      )}
                    </AnimatePresence>
                  </section>
                </Card>
                <p
                  aria-live="polite"
                  className={`relative z-10 mt-3 min-h-5 text-center text-sm ${
                    creationFeedback ? "visible" : "invisible"
                  } ${
                    availability === false || (name && !validName)
                      ? "text-destructive"
                      : "text-[color:var(--buzz-onboarding-backup-ink)]"
                  }`}
                  id="hosted-community-feedback"
                >
                  {creationFeedback ?? "Community address status"}
                </p>
              </>
            ) : (
              <>
                {renderCreationForm(false)}
                <p
                  aria-live="polite"
                  className={`relative z-10 mt-3 min-h-5 text-center text-sm ${
                    creationFeedback ? "visible" : "invisible"
                  } ${
                    availability === false || (name && !validName)
                      ? "text-destructive"
                      : "text-[color:var(--buzz-onboarding-backup-ink)]"
                  }`}
                  id="hosted-community-feedback"
                >
                  {creationFeedback ?? "Community address status"}
                </p>
              </>
            )}
          </>
        ) : (
          <Card
            aria-hidden
            className={`${FUZZY_SURFACE_CLASS} opacity-70`}
            variant="textured"
          />
        )}
      </div>

      {!modalOpen ? (
        <OnboardingFooter>
          <Button
            className={PAGE_CTA_CLASS}
            disabled={
              !validName ||
              availability === false ||
              checkingName ||
              busy ||
              atCommunityLimit
            }
            form="hosted-community-create-form"
            type="submit"
          >
            {busy ? <LoaderCircle className="h-4 w-4 animate-spin" /> : null}
            {action ?? "Next"}
          </Button>
          <Button
            className={PAGE_BACK_CLASS}
            disabled={busy}
            onClick={goBack}
            type="button"
            variant="ghost"
          >
            Back
          </Button>
        </OnboardingFooter>
      ) : null}

      {signInDialog}
    </div>
  );
}
