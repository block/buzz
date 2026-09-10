import * as React from "react";

import type { JoinPolicy } from "@/shared/api/invites";
import { resetAvatarPresentations } from "@/features/profile/avatarPresentationStore";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import {
  ONBOARDING_PREVIEW_LANDING_ACTIONS,
  type OnboardingPreviewPage,
  type OnboardingPreviewVariant,
  resolveOnboardingPreviewJourney,
} from "../onboardingPreview";
import { BackupStep } from "./BackupStep";
import {
  resetEncryptedBackupSession,
  useEncryptedBackupSession,
} from "./EncryptedBackupCreator";
import {
  HARNESS_CONNECTION_OPTIONS,
  HarnessConnectionDetailPreview,
  HarnessConnectionHelpPreview,
  HarnessConnectionMethodPreview,
  type HarnessConnectionMethod,
  HarnessConnectionPreview,
  runtimeNeedsOnboardingConnection,
} from "./HarnessConnectionStep";
import { JoinPolicyNotice } from "./JoinPolicyNotice";
import { KeySignInPreview } from "./KeySignInPreview";
import {
  IdentityKeyHelpDialog,
  IdentityKeyHelpPreviewContent,
} from "./IdentityKeyHelpDialog";
import { LandingBees } from "./LandingBees";
import {
  ONBOARDING_LANDING_CTA_CLASS,
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { OnboardingFooter, OnboardingFooterProvider } from "./OnboardingFooter";
import { PreviewCredentialsFields } from "./OnboardingPreviewCredentials";
import {
  BackupPasswordPreview,
  CommunityChoicePreview,
  CommunityConnectingPreview,
  CommunityCreatePreview,
  CommunityEntryPreview,
  CommunityHomePreview,
  CommunityProfilePreview,
  type CommunityPreviewRoute,
  DefaultConfigPreview,
  PasswordReset,
  StarterTeamPreview,
  WelcomeChannelPreview,
} from "./OnboardingPreviewJourney";
import {
  OnboardingPreviewLayoutProvider,
  OnboardingPreviewStep,
  useOnboardingPreviewCardLayout,
} from "./OnboardingPreviewShell";
import { OnboardingPreviewControls } from "./OnboardingPreviewControls";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
import { SetupStepPreview } from "./SetupStepPreview";

const EMAIL_SIGNUP_POLICY: JoinPolicy = {
  ageAttestationRequired: true,
  privacyMarkdown: "available",
  termsMarkdown: "available",
  version: "email-signup-workshop",
};

function Landing({
  onNavigate,
  variant,
}: {
  onNavigate: (page: OnboardingPreviewPage) => void;
  variant: OnboardingPreviewVariant;
}) {
  const isV3 = variant === "v3";
  const landingActions = ONBOARDING_PREVIEW_LANDING_ACTIONS[variant];

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell buzz-onboarding-welcome flex max-h-dvh items-start justify-center overflow-hidden px-4 py-8 text-foreground"
      data-testid="onboarding-preview-landing"
    >
      <StartupWindowDragRegion />
      <LandingBees />
      <OnboardingFooterProvider>
        <div className="relative my-auto flex w-full max-w-[720px] flex-col items-center text-center">
          <OnboardingSlideTransition
            className="flex w-full max-w-[720px] flex-col items-center text-center"
            transitionKey="preview-landing"
          >
            <img
              alt="Buzz"
              className="w-full max-w-[600px]"
              src="/landing/buzz-wordmark.png"
            />
            <p className="mt-2 max-w-[560px] text-center text-2xl font-normal leading-none text-foreground">
              Your people, your agents, your projects —<br />
              all in one place.
            </p>
            <div className="mt-10 flex flex-col items-center gap-3">
              <Button
                className={ONBOARDING_LANDING_CTA_CLASS}
                onClick={() => onNavigate(landingActions.primary.page)}
                type="button"
              >
                {landingActions.primary.label}
              </Button>
              <Button
                className={`${ONBOARDING_SECONDARY_CTA_CLASS} px-5`}
                onClick={() => onNavigate(landingActions.secondary.page)}
                type="button"
                variant="ghost"
              >
                {landingActions.secondary.label}
              </Button>
            </div>
            {isV3 ? (
              <OnboardingFooter className="max-w-none">
                <Button
                  className="h-auto text-foreground/70 hover:text-foreground"
                  data-testid="onboarding-preview-without-email"
                  onClick={() => onNavigate("backup-options")}
                  type="button"
                  variant="link"
                >
                  Start with a private key
                </Button>
              </OnboardingFooter>
            ) : (
              <IdentityKeyHelpDialog previewMode v3Presentation={false} />
            )}
          </OnboardingSlideTransition>
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}

function EmailSignup({
  password,
  onBack,
  onContinue,
  onPasswordChange,
  total,
}: {
  password: string;
  onBack: () => void;
  onContinue: (email: string) => void;
  onPasswordChange: (password: string) => void;
  total: number;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const [email, setEmail] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [ageConfirmed, setAgeConfirmed] = React.useState(false);
  const [agreementConfirmed, setAgreementConfirmed] = React.useState(false);
  const [openPolicyDocument, setOpenPolicyDocument] = React.useState<
    "terms" | "privacy" | null
  >(null);

  return (
    <OnboardingPreviewStep
      onBack={onBack}
      testId="onboarding-preview-email"
      total={total}
    >
      <OnboardingSlideTransition
        className={cn(
          "flex min-h-0 w-full max-w-[500px] flex-col",
          cardLayout ? "items-stretch" : "items-center",
        )}
        transitionKey="preview-email"
      >
        <h1 className="text-title font-normal text-foreground">
          Create an account
        </h1>
        <p className="mt-5 max-w-[440px] text-sm leading-6 text-foreground/80">
          Enter your email address to get started.
        </p>
        <form
          className="mt-8 w-full space-y-5 text-left"
          onSubmit={(event) => {
            event.preventDefault();
            onContinue(email);
          }}
        >
          <PreviewCredentialsFields
            email={email}
            emailId="onboarding-preview-email-address"
            onEmailChange={setEmail}
            onPasswordChange={onPasswordChange}
            confirmPassword={confirmPassword}
            onConfirmPasswordChange={setConfirmPassword}
            password={password}
            passwordAutoComplete="new-password"
            passwordId="onboarding-preview-password"
            passwordPlaceholder="Choose your password"
            showPasswordStrength
          />
          <JoinPolicyNotice
            ageConfirmed={ageConfirmed}
            agreementConfirmed={agreementConfirmed}
            onAgeConfirmedChange={setAgeConfirmed}
            onAgreementConfirmedChange={setAgreementConfirmed}
            onOpenDocument={setOpenPolicyDocument}
            policy={EMAIL_SIGNUP_POLICY}
            relayWsUrl="wss://preview.invalid"
            textTone="foreground"
          />
          <OnboardingFooter>
            <Button
              className={ONBOARDING_PRIMARY_CTA_CLASS}
              data-testid="onboarding-preview-email-continue"
              onClick={() => onContinue(email)}
              type="button"
            >
              Continue
            </Button>
          </OnboardingFooter>
        </form>
        {openPolicyDocument ? (
          <div
            aria-modal="true"
            className="fixed inset-0 z-[110] flex items-center justify-center bg-black/30 p-6 backdrop-blur-sm"
            role="dialog"
          >
            <div className="w-full max-w-lg rounded-2xl border border-foreground/15 bg-background p-6 text-left shadow-xl">
              <h2 className="text-xl font-medium text-foreground">
                {openPolicyDocument === "terms"
                  ? "Terms of Service"
                  : "Privacy Policy"}
              </h2>
              <p className="mt-3 text-sm leading-6 text-foreground/80">
                {openPolicyDocument === "terms"
                  ? "Review the terms that apply when you use Buzz."
                  : "Review how information is handled when you use Buzz."}
              </p>
              <Button
                className="mt-5"
                onClick={() => setOpenPolicyDocument(null)}
                type="button"
              >
                Close
              </Button>
            </div>
          </div>
        ) : null}
      </OnboardingSlideTransition>
    </OnboardingPreviewStep>
  );
}

function AgentHarnessStep({
  onBack,
  onNext,
}: {
  onBack: () => void;
  onNext: () => void;
}) {
  return (
    <OnboardingPreviewStep
      current={3}
      onBack={onBack}
      testId="onboarding-preview-setup"
    >
      <SetupStepPreview onBack={onBack} onNext={onNext} />
    </OnboardingPreviewStep>
  );
}

function EmailSignIn({
  email,
  onBack,
  onContinue,
  onEmailChange,
  onForgotPassword,
  onSignInWithKey,
  total,
}: {
  email: string;
  onBack: () => void;
  onContinue: () => void;
  onEmailChange: (email: string) => void;
  onForgotPassword: () => void;
  onSignInWithKey: () => void;
  total: number;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const [password, setPassword] = React.useState("");
  const canContinue = email.trim().length > 0 && password.length > 0;

  return (
    <OnboardingPreviewStep
      onBack={onBack}
      testId="onboarding-preview-sign-in"
      total={total}
    >
      <OnboardingSlideTransition
        className={cn(
          "flex min-h-0 w-full max-w-[500px] flex-col",
          cardLayout ? "items-stretch" : "items-center",
        )}
        transitionKey="preview-sign-in"
      >
        <h1 className="text-title font-normal text-foreground">Sign in</h1>
        <p className="mt-5 max-w-[440px] text-sm leading-6 text-foreground/80">
          Enter your email address and password to sign in.
        </p>
        <form
          className="mt-8 w-full space-y-5 text-left"
          onSubmit={(event) => {
            event.preventDefault();
            if (canContinue) onContinue();
          }}
        >
          <PreviewCredentialsFields
            email={email}
            emailId="onboarding-preview-sign-in-email"
            onEmailChange={onEmailChange}
            onPasswordChange={setPassword}
            password={password}
            passwordAutoComplete="current-password"
            passwordHelp={
              <Button
                className="h-auto justify-start p-0 text-xs font-medium text-foreground/70 no-underline hover:text-foreground hover:underline focus-visible:text-foreground focus-visible:underline"
                onClick={onForgotPassword}
                type="button"
                variant="link"
              >
                Forgot password?
              </Button>
            }
            passwordId="onboarding-preview-sign-in-password"
            passwordPlaceholder="Enter your password"
          />
          <div className="flex w-full items-baseline gap-1.5 pt-1 text-sm text-foreground/70">
            <span>or</span>
            <Button
              className="h-auto p-0 text-sm text-foreground"
              data-testid="onboarding-preview-sign-in-with-key"
              onClick={onSignInWithKey}
              type="button"
              variant="link"
            >
              Sign in with private key
            </Button>
          </div>
        </form>
        <OnboardingFooter>
          <Button
            className={ONBOARDING_PRIMARY_CTA_CLASS}
            data-testid="onboarding-preview-sign-in-submit"
            disabled={!canContinue}
            onClick={onContinue}
            type="button"
          >
            Sign in
          </Button>
        </OnboardingFooter>
      </OnboardingSlideTransition>
    </OnboardingPreviewStep>
  );
}

export function OnboardingPreviewApp() {
  const [run, setRun] = React.useState(0);
  const [variant, setVariant] = React.useState<OnboardingPreviewVariant>("v3");
  const [page, setPage] = React.useState<OnboardingPreviewPage>("landing");
  const [signInEmail, setSignInEmail] = React.useState("");
  const [signupPassword, setSignupPassword] = React.useState("");
  const [passwordResetEmail, setPasswordResetEmail] = React.useState("");
  const [installedHarnessIds, setInstalledHarnessIds] = React.useState(
    () =>
      new Set(
        HARNESS_CONNECTION_OPTIONS.filter(
          ({ runtime }) => runtime.availability === "available",
        ).map(({ runtime }) => runtime.id),
      ),
  );
  const [selectedHarnessId, setSelectedHarnessId] = React.useState("claude");
  const [harnessConnectionMethod, setHarnessConnectionMethod] =
    React.useState<HarnessConnectionMethod>("subscription");
  const [harnessConnectionInOnboarding, setHarnessConnectionInOnboarding] =
    React.useState(true);
  const [
    chooseHarnessConnectionMethodFirst,
    setChooseHarnessConnectionMethodFirst,
  ] = React.useState(true);
  const [harnessDetailBackPage, setHarnessDetailBackPage] =
    React.useState<OnboardingPreviewPage>("harness-connection");
  const [setupBackPage, setSetupBackPage] =
    React.useState<OnboardingPreviewPage>("landing");
  const [communityRoute, setCommunityRoute] =
    React.useState<CommunityPreviewRoute>("join");
  const [communityName, setCommunityName] = React.useState("Block Community");
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const backupSession = useEncryptedBackupSession();
  const journey = resolveOnboardingPreviewJourney(
    variant,
    harnessConnectionInOnboarding,
  );
  const selectedHarness =
    HARNESS_CONNECTION_OPTIONS.find(
      ({ runtime }) => runtime.id === selectedHarnessId,
    ) ?? HARNESS_CONNECTION_OPTIONS[0];

  React.useEffect(() => {
    if (page === "landing") {
      setPasswordResetEmail("");
      setSignInEmail("");
      setSignupPassword("");
    }
  }, [page]);

  const restart = React.useCallback(() => {
    resetAvatarPresentations();
    resetEncryptedBackupSession(backupSession);
    setPage("landing");
    setPasswordResetEmail("");
    setInstalledHarnessIds(
      new Set(
        HARNESS_CONNECTION_OPTIONS.filter(
          ({ runtime }) => runtime.availability === "available",
        ).map(({ runtime }) => runtime.id),
      ),
    );
    setSelectedHarnessId("claude");
    setHarnessConnectionMethod("subscription");
    setHarnessDetailBackPage("harness-connection");
    setRun((current) => current + 1);
    setSignInEmail("");
    setSignupPassword("");
    setCommunityRoute("join");
    setCommunityName("Block Community");
    setDisplayName("");
    setAvatarUrl("");
  }, [backupSession]);

  const changeVariant = React.useCallback(
    (nextVariant: OnboardingPreviewVariant) => {
      setVariant(nextVariant);
      restart();
    },
    [restart],
  );

  const continueFromAccount = React.useCallback(
    (backPage: OnboardingPreviewPage) => {
      setSetupBackPage(backPage);
      setPage(
        variant === "v3" &&
          harnessConnectionInOnboarding &&
          chooseHarnessConnectionMethodFirst
          ? "harness-connection-method"
          : journey.afterAccount,
      );
    },
    [
      chooseHarnessConnectionMethodFirst,
      harnessConnectionInOnboarding,
      journey.afterAccount,
      variant,
    ],
  );

  let content: React.ReactNode;
  if (page === "landing") {
    content = <Landing key={run} onNavigate={setPage} variant={variant} />;
  } else if (page === "email") {
    content = (
      <EmailSignup
        password={signupPassword}
        onBack={() => setPage("landing")}
        onContinue={() => continueFromAccount("email")}
        onPasswordChange={setSignupPassword}
        total={journey.totalSteps}
      />
    );
  } else if (page === "identity-key") {
    content = (
      <OnboardingPreviewStep
        allowHorizontalActionOverflow={variant === "v3"}
        onBack={() => setPage(variant === "v3" ? "backup-options" : "landing")}
        testId="onboarding-preview-identity-key"
        total={journey.totalSteps}
      >
        <BackupStep
          direction="forward"
          lockedBackupCreated={backupSession.created}
          onNext={() => continueFromAccount("identity-key")}
          onOpenPasswordBackup={() => setPage("backup-password")}
          onShowOptions={() => {
            setSetupBackPage("identity-key");
            setPage("backup-options");
          }}
          optionsExpanded={false}
          previewMode
          returningFromSecurity={false}
          showPreviewBackupShortcut={false}
          v3Presentation={variant === "v3"}
        />
      </OnboardingPreviewStep>
    );
  } else if (page === "backup-options") {
    content = (
      <OnboardingPreviewStep
        onBack={() => setPage(variant === "v3" ? "landing" : "identity-key")}
        security={variant === "today"}
        testId="onboarding-preview-backup-options"
        total={journey.totalSteps}
      >
        <BackupStep
          direction="forward"
          lockedBackupCreated={backupSession.created}
          onNext={() => setPage("identity-key")}
          onOpenIdentityKeyHelp={() => setPage("identity-key-help")}
          onOpenPasswordBackup={() => setPage("backup-password")}
          onShowOptions={() => setPage("backup-options")}
          optionsExpanded
          previewMode
          returningFromSecurity={false}
          v3Presentation={variant === "v3"}
        />
      </OnboardingPreviewStep>
    );
  } else if (page === "identity-key-help") {
    content = (
      <OnboardingPreviewStep
        onBack={() => setPage("backup-options")}
        testId="onboarding-preview-identity-key-help"
        total={journey.totalSteps}
      >
        <OnboardingSlideTransition
          className="flex min-h-0 w-full max-w-[560px] flex-col items-stretch justify-start text-left"
          transitionKey="preview-identity-key-help"
        >
          <IdentityKeyHelpPreviewContent />
        </OnboardingSlideTransition>
      </OnboardingPreviewStep>
    );
  } else if (page === "backup-password") {
    content = (
      <BackupPasswordPreview
        onBack={() => setPage("identity-key")}
        onDone={() =>
          variant === "v3"
            ? continueFromAccount("identity-key")
            : setPage("identity-key")
        }
        session={backupSession}
        total={journey.totalSteps}
        v3Presentation={variant === "v3"}
      />
    );
  } else if (page === "sign-in") {
    content = (
      <EmailSignIn
        email={signInEmail}
        onBack={() => setPage("landing")}
        onContinue={() => continueFromAccount("sign-in")}
        onEmailChange={setSignInEmail}
        onForgotPassword={() => {
          setPasswordResetEmail(signInEmail);
          setPage("forgot-password");
        }}
        onSignInWithKey={() => setPage("sign-in-key")}
        total={journey.totalSteps}
      />
    );
  } else if (page === "forgot-password") {
    content = (
      <PasswordReset
        initialEmail={passwordResetEmail}
        onBack={() => setPage("sign-in")}
        total={journey.totalSteps}
      />
    );
  } else if (page === "sign-in-key") {
    content = (
      <KeySignInPreview
        onBack={() => setPage(variant === "v3" ? "sign-in" : "landing")}
        onContinue={() => continueFromAccount("sign-in-key")}
        total={journey.totalSteps}
      />
    );
  } else if (page === "setup") {
    content = (
      <AgentHarnessStep
        onBack={() => setPage(setupBackPage)}
        onNext={() => setPage("config")}
      />
    );
  } else if (page === "harness-connection-method") {
    content = (
      <HarnessConnectionMethodPreview
        onBack={() => setPage(setupBackPage)}
        onSelect={(method) => {
          setHarnessConnectionMethod(method);
          if (method === "api") {
            setSelectedHarnessId("buzz-agent");
            setHarnessDetailBackPage("harness-connection-method");
            setPage("harness-connection-detail");
          } else {
            setPage("harness-connection");
          }
        }}
        onSetUpLater={() => setPage("community-choice")}
        total={journey.totalSteps}
      />
    );
  } else if (page === "harness-connection") {
    content = (
      <HarnessConnectionPreview
        installedIds={installedHarnessIds}
        method={
          chooseHarnessConnectionMethodFirst
            ? harnessConnectionMethod
            : undefined
        }
        onBack={() =>
          setPage(
            chooseHarnessConnectionMethodFirst
              ? "harness-connection-method"
              : setupBackPage,
          )
        }
        onHelp={
          chooseHarnessConnectionMethodFirst
            ? undefined
            : () => setPage("harness-connection-help")
        }
        onSelect={(option) => {
          setSelectedHarnessId(option.runtime.id);
          setHarnessDetailBackPage("harness-connection");
          const nextMethod = chooseHarnessConnectionMethodFirst
            ? harnessConnectionMethod
            : (option.methods[0] ?? "api");
          setHarnessConnectionMethod(nextMethod);
          setPage(
            installedHarnessIds.has(option.runtime.id) &&
              !runtimeNeedsOnboardingConnection(nextMethod, option.runtime.id)
              ? "community-choice"
              : "harness-connection-detail",
          );
        }}
        total={journey.totalSteps}
      />
    );
  } else if (page === "harness-connection-help") {
    content = (
      <HarnessConnectionHelpPreview
        onBack={() => setPage("harness-connection")}
        total={journey.totalSteps}
      />
    );
  } else if (page === "harness-connection-detail") {
    content = (
      <HarnessConnectionDetailPreview
        installed={installedHarnessIds.has(selectedHarness.runtime.id)}
        lockMethod={chooseHarnessConnectionMethodFirst}
        method={harnessConnectionMethod}
        onBack={() => setPage(harnessDetailBackPage)}
        onCheckAgain={() => {
          setInstalledHarnessIds((current) =>
            new Set(current).add(selectedHarness.runtime.id),
          );
          if (
            !runtimeNeedsOnboardingConnection(
              harnessConnectionMethod,
              selectedHarness.runtime.id,
            )
          ) {
            setPage("community-choice");
          }
        }}
        onContinue={() => setPage("community-choice")}
        onMethodChange={setHarnessConnectionMethod}
        onUseDifferentHarness={
          chooseHarnessConnectionMethodFirst &&
          harnessConnectionMethod === "api" &&
          selectedHarness.runtime.id === "buzz-agent"
            ? () => setPage("harness-connection")
            : undefined
        }
        option={selectedHarness}
        total={journey.totalSteps}
      />
    );
  } else if (page === "config") {
    content = (
      <DefaultConfigPreview
        onBack={() => setPage("setup")}
        onNext={() => setPage("community-choice")}
      />
    );
  } else if (page === "community-choice") {
    content = (
      <CommunityChoicePreview
        current={journey.communityStep}
        includeExistingCommunity={journey.includeExistingCommunity}
        onBack={() => setPage(journey.communityChoiceBack ?? setupBackPage)}
        onChoose={(route) => {
          setCommunityRoute(route);
          setPage("community-entry");
        }}
        total={journey.totalSteps}
      />
    );
  } else if (page === "community-entry") {
    content =
      communityRoute === "create" ? (
        <CommunityCreatePreview
          current={journey.communityStep}
          onBack={() => setPage("community-choice")}
          onContinue={(name) => {
            setCommunityName(name);
            setPage(journey.afterCommunityEntry);
          }}
          total={journey.totalSteps}
        />
      ) : (
        <CommunityEntryPreview
          current={journey.communityStep}
          onBack={() => setPage("community-choice")}
          onContinue={(name) => {
            setCommunityName(name);
            setPage(journey.afterCommunityEntry);
          }}
          previewVariant={variant}
          route={communityRoute}
          total={journey.totalSteps}
        />
      );
  } else if (page === "community-connecting") {
    content = (
      <CommunityConnectingPreview
        communityName={communityName}
        onBack={() => setPage("community-entry")}
        onContinue={() => setPage("community-profile")}
      />
    );
  } else if (page === "community-profile") {
    content = (
      <CommunityProfilePreview
        avatarUrl={avatarUrl}
        current={journey.profileStep}
        displayName={displayName}
        onAvatarUrlChange={setAvatarUrl}
        onBack={() => setPage("community-entry")}
        onDisplayNameChange={setDisplayName}
        onNext={() => setPage(journey.afterProfile)}
        total={journey.totalSteps}
      />
    );
  } else if (page === "starter-team") {
    content = (
      <StarterTeamPreview
        onBack={() => setPage("community-profile")}
        onNext={() => setPage("welcome-channel")}
      />
    );
  } else if (page === "welcome-channel") {
    content = (
      <WelcomeChannelPreview
        communityName={communityName}
        onBack={() => setPage("starter-team")}
        onNext={() => setPage("community-home")}
      />
    );
  } else {
    content = (
      <CommunityHomePreview
        avatarUrl={avatarUrl}
        communityName={communityName}
        displayName={displayName}
      />
    );
  }

  return (
    <>
      <OnboardingPreviewControls
        chooseHarnessConnectionMethodFirst={chooseHarnessConnectionMethodFirst}
        harnessConnectionInOnboarding={harnessConnectionInOnboarding}
        onChooseHarnessConnectionMethodFirstChange={(enabled) => {
          setChooseHarnessConnectionMethodFirst(enabled);
          restart();
        }}
        onHarnessConnectionInOnboardingChange={(included) => {
          setHarnessConnectionInOnboarding(included);
          restart();
        }}
        onRestart={restart}
        onVariantChange={changeVariant}
        variant={variant}
      />
      <OnboardingPreviewLayoutProvider
        card={
          variant === "v3" && page !== "landing" && page !== "community-home"
        }
      >
        {content}
      </OnboardingPreviewLayoutProvider>
    </>
  );
}
