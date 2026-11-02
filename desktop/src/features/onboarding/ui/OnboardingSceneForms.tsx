import type * as React from "react";

import {
  BusinessMark,
  FirstRunButton,
  FirstRunDivider,
  FirstRunError,
  FirstRunLink,
  FirstRunPasswordField,
} from "./firstRunControls";
import { FirstRunHead } from "./FirstRunFrame";
import {
  ChevronRightIcon,
  GlobeIcon,
  PlusIcon,
  RingIcon,
} from "./firstRunIcons";
import type { PresentationProps } from "./OnboardingSceneTypes";
import type { OnboardingBusinessChoice } from "./OnboardingSceneTypes";

function GoogleAccountAction({
  data,
  onGoogleSignIn,
  googlePending,
}: Pick<PresentationProps, "data" | "onGoogleSignIn" | "googlePending">) {
  if (!onGoogleSignIn) return null;
  return (
    <>
      <FirstRunDivider>or</FirstRunDivider>
      <FirstRunButton
        data-testid="account-auth-google"
        disabled={data.pending}
        onClick={onGoogleSignIn}
      >
        <GlobeIcon size={18} />
        {googlePending ? "Waiting for Google…" : "Continue with Google"}
      </FirstRunButton>
      {googlePending ? (
        <p className="c3fr-note" role="status">
          Finish signing in in your browser. You can cancel there and return
          here.
        </p>
      ) : null}
    </>
  );
}

function AdvancedLink({ onAdvanced }: Pick<PresentationProps, "onAdvanced">) {
  if (!onAdvanced) return null;
  return (
    <FirstRunLink
      className="c3fr-foot-link"
      data-testid="account-auth-advanced"
      onClick={onAdvanced}
    >
      Advanced sign-in options
    </FirstRunLink>
  );
}

export function AccountForm({
  data,
  error,
  headingRef,
  inviteBrand,
  onAdvanced,
  onEmailChange,
  onNameChange,
  onPasswordChange,
  onSubmit,
  onGoogleSignIn,
  googlePending,
}: PresentationProps) {
  return (
    <form className="c3fr-form" onSubmit={onSubmit}>
      {inviteBrand}
      <FirstRunHead
        headingRef={headingRef}
        lede={
          inviteBrand
            ? "Create an account to join your team’s workspace on Colony."
            : "Create an account. Make room for what’s next."
        }
        title={inviteBrand ? "You’re invited." : "Let’s get you started."}
      />
      <div className="c3fr-field-group">
        <label className="c3fr-label" htmlFor="full-name">
          Your name
        </label>
        <input
          autoComplete="name"
          className="c3fr-input"
          defaultValue={data.visualOnly ? undefined : data.name}
          id="full-name"
          name="full-name"
          onChange={(event) => onNameChange?.(event.currentTarget.value)}
          required
          value={data.visualOnly ? data.name : undefined}
        />
      </div>
      <div className="c3fr-field-group">
        <label className="c3fr-label" htmlFor="account-email">
          Email address
        </label>
        <input
          autoComplete="email"
          className="c3fr-input"
          defaultValue={data.visualOnly ? undefined : data.email}
          id="account-email"
          name="email"
          onChange={(event) => onEmailChange?.(event.currentTarget.value)}
          placeholder="you@business.com"
          required
          type="email"
          value={data.visualOnly ? data.email : undefined}
        />
      </div>
      <FirstRunPasswordField
        autoComplete="new-password"
        id="account-password"
        label="Password"
        minLength={10}
        onChange={(event) => onPasswordChange?.(event.currentTarget.value)}
        placeholder="At least 10 characters"
      />
      {error ? <FirstRunError>{error}</FirstRunError> : null}
      <FirstRunButton
        data-testid="account-auth-submit-signup"
        disabled={data.pending}
        kind="primary"
        type="submit"
      >
        {data.pending ? "Please wait…" : "Create account"}
      </FirstRunButton>
      <GoogleAccountAction
        data={data}
        googlePending={googlePending}
        onGoogleSignIn={onGoogleSignIn}
      />
      <AdvancedLink onAdvanced={onAdvanced} />
    </form>
  );
}

export function SignInForm({
  data,
  error,
  headingRef,
  inviteBrand,
  onAdvanced,
  onEmailChange,
  onNavigate,
  onPasswordChange,
  onSubmit,
  onGoogleSignIn,
  googlePending,
}: PresentationProps) {
  return (
    <form aria-label="Sign in" className="c3fr-form" onSubmit={onSubmit}>
      {inviteBrand}
      <FirstRunHead
        headingRef={headingRef}
        lede={
          inviteBrand
            ? "Sign in to join your team’s workspace on Colony."
            : "Let’s pick up where you left off."
        }
        title="Welcome back."
      />
      <div className="c3fr-field-group">
        <label className="c3fr-label" htmlFor="signin-email">
          Email address
        </label>
        <input
          autoComplete="email"
          className="c3fr-input"
          defaultValue={data.visualOnly ? undefined : data.email}
          id="signin-email"
          onChange={(event) => onEmailChange?.(event.currentTarget.value)}
          required
          type="email"
          value={data.visualOnly ? data.email : undefined}
        />
      </div>
      <FirstRunPasswordField
        autoComplete="current-password"
        defaultValue={data.visualOnly ? "colony-demo-only" : undefined}
        id="signin-password"
        label="Password"
        labelAside={
          <FirstRunLink
            className="forgot-link"
            onClick={() => onNavigate?.("forgot")}
          >
            Forgot password?
          </FirstRunLink>
        }
        onChange={(event) => onPasswordChange?.(event.currentTarget.value)}
      />
      {error ? <FirstRunError>{error}</FirstRunError> : null}
      <FirstRunButton
        data-testid="account-auth-submit-signin"
        disabled={data.pending}
        kind="primary"
        type="submit"
      >
        {data.pending ? "Please wait…" : "Sign in"}
      </FirstRunButton>
      <GoogleAccountAction
        data={data}
        googlePending={googlePending}
        onGoogleSignIn={onGoogleSignIn}
      />
      <div className="c3fr-foot">
        <span className="c3fr-foot-line">
          New to Colony?{" "}
          <FirstRunLink onClick={() => onNavigate?.("account")}>
            Create an account
          </FirstRunLink>
        </span>
        <AdvancedLink onAdvanced={onAdvanced} />
      </div>
    </form>
  );
}

export function BusinessForm({
  scene,
  data,
  error,
  headingRef,
  onBusinessChange,
  onDescriptionChange,
  onLogoChange,
  onLogoError,
  onNavigate,
  onReadWebsite,
  onSubmit,
  onWebsiteChange,
  canSubmit,
  pending,
}: PresentationProps) {
  const another =
    scene === "additional" || (data.visualOnly && data.business === "");
  const logoTitle = data.logoUrl
    ? data.logoSource === "website"
      ? "From your website"
      : "Your uploaded logo"
    : "Using your initial for now";
  return (
    <form
      className="c3fr-form"
      onSubmit={onSubmit}
      style={{ "--c3fr-gap": 20 } as React.CSSProperties}
    >
      <FirstRunHead
        headingRef={headingRef}
        lede={
          another
            ? "An idea or an existing business. Start with the essentials."
            : "A few details so Scout and the team start in the right place."
        }
        title={another ? "Your next business." : "What are you building?"}
      />
      <div className="c3fr-field-group">
        <label className="c3fr-label" htmlFor="business-name">
          Business name
        </label>
        <input
          autoComplete="organization"
          className="c3fr-input"
          id="business-name"
          maxLength={200}
          onChange={(event) => onBusinessChange?.(event.currentTarget.value)}
          placeholder="A working name is fine"
          required
          value={data.business}
        />
      </div>
      <div className="c3fr-field-group">
        <label className="c3fr-label" htmlFor="business-website">
          <span>
            Website <small>· optional</small>
          </span>
        </label>
        <div className="c3fr-row">
          <input
            autoComplete="url"
            className="c3fr-input"
            id="business-website"
            inputMode="url"
            onChange={(event) => onWebsiteChange?.(event.currentTarget.value)}
            placeholder="yourbusiness.co.za"
            value={data.website}
          />
          <button
            aria-busy={data.websitePending}
            className="c3fr-button c3fr-inline-button"
            disabled={data.websitePending}
            onClick={onReadWebsite}
            type="button"
          >
            {data.websitePending ? "Reading…" : "Read website"}
          </button>
        </div>
        <span className="c3fr-help">
          Scout can read it to fill in the rest for you.
        </span>
      </div>
      <div className="c3fr-field-group" id="business-logo-field">
        <span className="c3fr-label">
          <span>
            Logo <small>· optional</small>
          </span>
        </span>
        <input
          accept="image/png,image/jpeg,image/webp,image/svg+xml"
          aria-label="Upload business logo"
          className="c3fr-sr-only"
          id="business-logo"
          onChange={(event) =>
            onLogoChange?.(event.currentTarget.files?.[0] ?? null)
          }
          type="file"
        />
        <div className="c3fr-logo-box">
          <BusinessMark
            logoUrl={data.logoUrl}
            name={data.business}
            onLogoError={onLogoError}
            size="small"
          />
          <span className="c3fr-logo-text">
            <span className="c3fr-logo-title">{logoTitle}</span>
            <span className="c3fr-help">PNG or SVG, square works best</span>
          </span>
          <label
            aria-label={
              data.logoUrl ? "Change business logo" : "Upload business logo"
            }
            className="c3fr-chip-button c3fr-logo-button"
            htmlFor="business-logo"
          >
            {data.logoUrl ? "Change logo" : "Upload logo"}
          </label>
        </div>
      </div>
      <div className="c3fr-field-group">
        <label className="c3fr-label" htmlFor="business-description">
          What does your business do?
        </label>
        <textarea
          className="c3fr-textarea"
          id="business-description"
          maxLength={4000}
          onChange={(event) => onDescriptionChange?.(event.currentTarget.value)}
          placeholder="We help…"
          required
          rows={3}
          value={data.description}
        />
      </div>
      {error ? <FirstRunError>{error}</FirstRunError> : null}
      <div className="c3fr-actions">
        <FirstRunButton
          onClick={() =>
            onNavigate?.(scene === "additional" ? "workspace" : "account")
          }
        >
          {scene === "additional" ? "Cancel" : "Back"}
        </FirstRunButton>
        <FirstRunButton
          disabled={(pending ?? data.pending) || canSubmit === false}
          kind="primary"
          type="submit"
        >
          {pending || data.pending ? "Creating…" : "Continue"}
        </FirstRunButton>
      </div>
    </form>
  );
}

/** The returning person's list of businesses ("Where shall we go?"). */
export function BusinessChooser({
  data,
  businessChoices,
  error,
  headingRef,
  onNavigate,
  onCreateBusiness,
  onSelectBusiness,
  pending = false,
}: {
  data: PresentationProps["data"];
  businessChoices?: OnboardingBusinessChoice[];
  error?: string | null;
  headingRef?: PresentationProps["headingRef"];
  onNavigate?: PresentationProps["onNavigate"];
  onCreateBusiness?: () => void;
  onSelectBusiness?: (id: string) => void;
  /** A business was picked and its AI connection is being checked. */
  pending?: boolean;
}) {
  const choices: OnboardingBusinessChoice[] =
    businessChoices ??
    (data.visualOnly
      ? [
          {
            id: "studio",
            name: data.business || "Your business",
            role: "Owner",
            lastVisited: "today",
          },
          {
            id: "rosebank",
            name: "Rosebank Studio",
            role: "Team member",
            lastVisited: "3 days ago",
          },
        ]
      : []);
  return (
    <div
      className="c3fr-form"
      style={{ "--c3fr-gap": 24 } as React.CSSProperties}
    >
      <FirstRunHead
        headingRef={headingRef}
        lede="Choose a business to pick up your work."
        title="Where shall we go?"
      />
      {error ? <FirstRunError>{error}</FirstRunError> : null}
      <ul
        aria-busy={pending}
        aria-label="Your businesses"
        className="c3fr-business-list"
        data-testid="onboarding-business-list"
      >
        {choices.map((choice, index) => (
          <li key={choice.id}>
            <button
              className="c3fr-business"
              data-current={index === 0 ? "true" : undefined}
              data-testid={`onboarding-business-${choice.id}`}
              disabled={pending}
              onClick={() => onSelectBusiness?.(choice.id)}
              type="button"
            >
              <BusinessMark
                dot={index === 0 ? "#f5a4d5" : undefined}
                logoUrl={choice.logoUrl}
                name={choice.name}
                relayUrl={choice.relayUrl}
                communityId={choice.id}
                tone={index === 0 ? undefined : "light"}
              />
              <span className="c3fr-business-text">
                <strong>{choice.name}</strong>
                <small>
                  {choice.role}
                  {choice.lastVisited
                    ? ` · last visited ${choice.lastVisited}`
                    : ""}
                </small>
              </span>
              <ChevronRightIcon size={18} strokeWidth={2.4} />
            </button>
          </li>
        ))}
      </ul>
      <p className="c3fr-checking" role="status">
        <span
          className="c3fr-ring"
          data-spinning={pending ? "true" : undefined}
        >
          <RingIcon size={16} />
        </span>
        <span
          data-testid={pending ? "onboarding-business-checking" : undefined}
        >
          {pending
            ? "Checking your AI connection"
            : "When you pick a business, Colony checks its AI connection first."}
        </span>
      </p>
      <button
        className="c3fr-dashed-button"
        data-testid="onboarding-create-business"
        onClick={onCreateBusiness ?? (() => onNavigate?.("additional"))}
        type="button"
      >
        <PlusIcon size={18} strokeWidth={2.4} />
        Start another business
      </button>
    </div>
  );
}
