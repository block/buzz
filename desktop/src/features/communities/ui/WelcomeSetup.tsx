import * as React from "react";

import {
  listOwnedBusinesses,
  type SelfServeCommunity,
} from "@/features/communities/selfProvisioningApi";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { useDeviceAiConnectionCheck } from "@/features/onboarding/deviceAiConnection";
import {
  businessCommunityRelayUrl,
  ownedBusinessProfile,
  readOnboardingBusinessProfile,
  type OnboardingBusinessProfile,
} from "@/features/onboarding/ui/BusinessSetupStep";
import { ConnectSetupStep } from "@/features/onboarding/ui/ConnectSetupStep";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import {
  BusinessMark,
  FirstRunButton,
  FirstRunError,
  FirstRunLink,
} from "@/features/onboarding/ui/firstRunControls";
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  LinkIcon,
  PlusIcon,
  RefreshIcon,
} from "@/features/onboarding/ui/firstRunIcons";
import { recordBusinessVisit } from "@/features/onboarding/businessVisits";
import { useIdentityQuery } from "@/shared/api/hooks";

import { AddCommunityPanel } from "./AddCommunityPanel";
import { CommunityFieldScreen, CommunityFrame } from "./CommunityFrame";

type WelcomeSetupPage = "welcome" | "add" | "connect";
type WelcomeSetupInitialPage =
  | WelcomeSetupPage
  | "existing"
  | "join"
  | "member"
  | "owned"
  | "create";

type WelcomeSetupProps = {
  initialPage?: WelcomeSetupInitialPage;
  onBack?: () => void;
};

function pageFor(initial: WelcomeSetupInitialPage): WelcomeSetupPage {
  if (initial === "join" || initial === "create") return "add";
  if (initial === "existing" || initial === "member" || initial === "owned")
    return "welcome";
  return initial;
}

function hostOf(business: SelfServeCommunity): string {
  return business.normalized_host || business.slug;
}

/**
 * "Join or create a community": the first screen when this computer has no
 * community yet. Join and Create open the Add a community card; "I already
 * have a community" reconnects in place, as an owner or as a member.
 */
export function WelcomeSetup({
  initialPage = "welcome",
  onBack,
}: WelcomeSetupProps) {
  const [page, setPage] = React.useState<WelcomeSetupPage>(
    pageFor(initialPage),
  );
  const [role, setRole] = React.useState<"owner" | "member">(
    initialPage === "member" ? "member" : "owner",
  );
  const [ownedBusinesses, setOwnedBusinesses] = React.useState<
    SelfServeCommunity[]
  >([]);
  const [businessListLoading, setBusinessListLoading] = React.useState(false);
  const [businessListError, setBusinessListError] = React.useState<
    string | null
  >(null);
  const [joinError, setJoinError] = React.useState<string | null>(null);
  const [connectTarget, setConnectTarget] = React.useState<{
    business: SelfServeCommunity;
    profile: OnboardingBusinessProfile;
  } | null>(null);
  const [connectError, setConnectError] = React.useState<string | null>(null);
  const communityOnboarding = useCommunityOnboarding();
  // Opening an owned business on a device with no working AI goes through
  // Connect first, the same as the first-run business chooser.
  const deviceConnection = useDeviceAiConnectionCheck(
    page === "welcome" && role === "owner",
  );
  const identityQuery = useIdentityQuery();
  const headingRef = React.useRef<HTMLHeadingElement>(null);
  // The start page takes focus on its heading so a screen reader lands on it.
  React.useEffect(() => {
    if (page === "welcome") headingRef.current?.focus();
  }, [page]);

  const startConnection = React.useCallback(
    (relayUrl: string) => {
      communityOnboarding.start({
        source: "first-community",
        firstCommunityPage: role === "member" ? "member" : "join",
        relayUrl,
      });
    },
    [communityOnboarding, role],
  );

  const redeemInvite = React.useCallback(
    (relayUrl: string, code: string, policyReceipt?: string) => {
      communityOnboarding.start({
        source: "first-community",
        firstCommunityPage: role === "member" ? "member" : "join",
        relayUrl,
        inviteCode: code,
        policyReceipt,
      });
    },
    [communityOnboarding, role],
  );

  const loadOwnedBusinesses = React.useCallback(async () => {
    const pubkey = identityQuery.data?.pubkey;
    if (!pubkey) {
      setBusinessListError("Could not load your businesses. Try again.");
      return;
    }
    setBusinessListLoading(true);
    setBusinessListError(null);
    try {
      setOwnedBusinesses(
        await listOwnedBusinesses(
          pubkey,
          (id) => readOnboardingBusinessProfile(id)?.name ?? null,
        ),
      );
    } catch {
      setBusinessListError("Could not load your businesses. Try again.");
    } finally {
      setBusinessListLoading(false);
    }
  }, [identityQuery.data?.pubkey]);

  // The owner list loads when the owner role is showing on the start page.
  React.useEffect(() => {
    if (page !== "welcome" || role !== "owner") return;
    if (!identityQuery.data?.pubkey) return;
    void loadOwnedBusinesses();
  }, [identityQuery.data?.pubkey, loadOwnedBusinesses, page, role]);

  const openOwnedBusiness = (business: SelfServeCommunity) => {
    const started = communityOnboarding.start({
      source: "first-community",
      firstCommunityPage: "owned",
      relayUrl: businessCommunityRelayUrl(business),
      businessCommunityId: business.id,
      communityName:
        readOnboardingBusinessProfile(business.id)?.name ||
        business.name ||
        business.slug,
    });
    if (started) recordBusinessVisit(business.id);
    return started;
  };

  const selectOwnedBusiness = (business: SelfServeCommunity) => {
    if (deviceConnection.checking) return;
    void deviceConnection.resolve().then((connected) => {
      if (connected === null) return;
      if (!connected) {
        setConnectTarget({ business, profile: ownedBusinessProfile(business) });
        setConnectError(null);
        setPage("connect");
        return;
      }
      if (!openOwnedBusiness(business)) {
        setBusinessListError(
          "Finish the business setup already in progress before opening this business.",
        );
      }
    });
  };

  if (page === "connect" && connectTarget) {
    return (
      <ConnectSetupStep
        autoPick
        business={connectTarget.profile}
        communityId={connectTarget.business.id}
        relayUrl={businessCommunityRelayUrl(connectTarget.business)}
        error={connectError}
        onBack={() => {
          setConnectError(null);
          setPage("welcome");
        }}
        onContinue={() => {
          if (!openOwnedBusiness(connectTarget.business)) {
            setConnectError(
              "Finish the business setup already in progress before opening this business.",
            );
          }
        }}
      />
    );
  }

  if (page === "add") {
    return (
      <CommunityFieldScreen testId="welcome-setup">
        <AddCommunityPanel
          joinError={joinError}
          onClose={() => {
            setJoinError(null);
            setPage("welcome");
          }}
          onStart={({ relayUrl, inviteCode, policyReceipt }) => {
            const started = communityOnboarding.start({
              source: "first-community",
              firstCommunityPage: "join",
              relayUrl,
              inviteCode,
              policyReceipt,
            });
            if (!started)
              setJoinError(
                "Finish connecting the community already in progress, then try again.",
              );
          }}
        />
      </CommunityFieldScreen>
    );
  }

  return (
    <CommunityFrame
      body="A community is your company’s home: its channels, its threads and the AI employees who work in them."
      heading="Where people and agents build together."
      testId="welcome-setup"
    >
      <div
        className="c3fr-brand-column"
        style={{ "--c3fr-gap": 26 } as React.CSSProperties}
      >
        {onBack ? (
          <FirstRunLink
            className="c3fr-back-link"
            data-testid="welcome-setup-back"
            onClick={onBack}
          >
            <ChevronLeftIcon size={16} strokeWidth={2.4} />
            Back
          </FirstRunLink>
        ) : null}
        <div className="c3fr-head">
          <h1 className="c3fr-page-title" ref={headingRef} tabIndex={-1}>
            Join or create a community
          </h1>
          <p className="c3fr-lede c3fr-muted-lede">
            Every community lives at its own address. Choose how you’d like to
            start.
          </p>
        </div>
        <div className="c3fr-gate-list">
          <button
            className="c3fr-gate-link"
            data-testid="community-choice-join"
            onClick={() => setPage("add")}
            type="button"
          >
            <span aria-hidden="true" className="c3fr-tile">
              <LinkIcon size={21} />
            </span>
            <span className="c3fr-gate-link-text">
              <span className="c3fr-gate-link-title">Join a community</span>
              <span className="c3fr-gate-link-desc">
                Use an invite link or a community address.
              </span>
            </span>
            <ChevronRightIcon size={18} strokeWidth={2.4} />
          </button>
          <button
            className="c3fr-gate-link"
            data-testid="community-choice-create"
            onClick={() => setPage("add")}
            type="button"
          >
            <span aria-hidden="true" className="c3fr-tile">
              <PlusIcon size={21} strokeWidth={2.2} />
            </span>
            <span className="c3fr-gate-link-text">
              <span className="c3fr-gate-link-title">Create a community</span>
              <span className="c3fr-gate-link-desc">
                Claim a Colony address for your team.
              </span>
            </span>
            <ChevronRightIcon size={18} strokeWidth={2.4} />
          </button>
          <section
            className="c3fr-reconnect"
            data-testid="community-choice-existing"
          >
            <div className="c3fr-reconnect-head">
              <span aria-hidden="true" className="c3fr-tile" data-tone="dark">
                <RefreshIcon size={21} />
              </span>
              <span className="c3fr-gate-link-text">
                <span className="c3fr-gate-link-title">
                  I already have a community
                </span>
                <span className="c3fr-gate-link-desc">
                  Reconnect to your community.
                </span>
              </span>
            </div>
            <fieldset className="c3fr-roles">
              <legend className="c3fr-sr-only">How you belong</legend>
              <label data-selected={role === "owner" ? "true" : undefined}>
                <input
                  checked={role === "owner"}
                  data-testid="existing-choice-owner"
                  name="belong"
                  onChange={() => setRole("owner")}
                  type="radio"
                />
                I own the business
              </label>
              <label data-selected={role === "member" ? "true" : undefined}>
                <input
                  checked={role === "member"}
                  data-testid="existing-choice-member"
                  name="belong"
                  onChange={() => setRole("member")}
                  type="radio"
                />
                I’m a member or admin
              </label>
            </fieldset>
            {role === "owner" ? (
              <div className="c3fr-reconnect-list">
                <span className="c3fr-eyebrow c3fr-eyebrow-muted">
                  Your businesses
                </span>
                {businessListLoading ? (
                  <p className="c3fr-note" role="status">
                    Loading your businesses…
                  </p>
                ) : businessListError ? (
                  <div role="alert">
                    <FirstRunError>{businessListError}</FirstRunError>
                    <FirstRunButton
                      fit="content"
                      onClick={() => void loadOwnedBusinesses()}
                      size="small"
                    >
                      Try again
                    </FirstRunButton>
                  </div>
                ) : ownedBusinesses.length === 0 ? (
                  <div className="c3fr-reconnect-empty">
                    <p className="c3fr-note">
                      This account doesn’t own a business yet.
                    </p>
                    <FirstRunButton
                      fit="content"
                      onClick={() => setPage("add")}
                      size="small"
                    >
                      Create a business
                    </FirstRunButton>
                  </div>
                ) : (
                  ownedBusinesses.map((business) => (
                    <div className="c3fr-reconnect-row" key={business.id}>
                      <BusinessMark
                        communityId={business.id}
                        dot="#f5a4d5"
                        name={business.name || business.slug}
                        relayUrl={businessCommunityRelayUrl(business)}
                        size="compact"
                      />
                      <span className="c3fr-reconnect-text">
                        <strong>{business.name || business.slug}</strong>
                        <small>Owner · {hostOf(business)}</small>
                      </span>
                      <button
                        className="c3fr-reconnect-button"
                        data-testid={`welcome-owned-business-${business.id}`}
                        disabled={deviceConnection.checking}
                        onClick={() => selectOwnedBusiness(business)}
                        type="button"
                      >
                        Reconnect
                      </button>
                    </div>
                  ))
                )}
                {deviceConnection.checking ? (
                  <p
                    className="c3fr-checking"
                    data-testid="welcome-owned-business-checking"
                    role="status"
                  >
                    <span className="c3fr-ring" data-spinning="true">
                      <RefreshIcon size={16} />
                    </span>
                    Checking your AI connection
                  </p>
                ) : null}
              </div>
            ) : null}
            <InviteRedeemForm
              error={joinError}
              isRedeeming={false}
              onCancel={() => setRole("owner")}
              onConnect={startConnection}
              onRedeem={redeemInvite}
              placeholder="https://"
              variant="onboarding-spotlight"
            />
          </section>
        </div>
      </div>
    </CommunityFrame>
  );
}
