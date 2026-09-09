import {
  IconArrowLeft,
  IconBan,
  IconDeviceFloppy,
  IconPlus,
  IconShieldCheck,
} from "@tabler/icons-react";
import { type ReactNode, useState } from "react";

import { Button } from "@/shared/ui/Button";
import { CodeInput } from "@/shared/ui/CodeInput";
import { IconButton } from "@/shared/ui/IconButton";
import { Panel } from "@/shared/ui/Panel";
import { TextField } from "@/shared/ui/TextField";

function OnboardingActions() {
  return (
    <div className="onboarding-actions">
      <IconButton
        aria-label="Go back"
        icon={<IconArrowLeft size={20} stroke={1.7} aria-hidden="true" />}
        variant="quiet"
        size="large"
        shape="round"
      />
      <Button variant="primary">Continue</Button>
    </div>
  );
}

function LegalCopy() {
  return (
    <p className="onboarding-legal text-body-sm text-tertiary">
      By continuing, you confirm you are 18 years of age or older and agree to
      the Buzz <a href="#terms">Terms of Service</a> and{" "}
      <a href="#privacy">Privacy Policy</a>
    </p>
  );
}

function OnboardingFrame({
  children,
  legal = false,
}: {
  children: ReactNode;
  legal?: boolean;
}) {
  return (
    <div className="onboarding-scene">
      <div className="onboarding-window-controls" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>
      <div className="onboarding-card-shell">
        <Panel>{children}</Panel>
      </div>
      {legal ? <LegalCopy /> : null}
    </div>
  );
}

function AccountStage() {
  const [firstName, setFirstName] = useState("");
  const [lastName, setLastName] = useState("");
  const [email, setEmail] = useState("");

  return (
    <OnboardingFrame legal>
      <h2 className="text-title text-primary">Create a Buzz account</h2>
      <div className="onboarding-fields">
        <div className="onboarding-field-row">
          <TextField
            label="First name"
            placeholder="Your name"
            value={firstName}
            onValueChange={setFirstName}
          />
          <TextField
            label="Last name"
            placeholder="Your last name"
            value={lastName}
            onValueChange={setLastName}
          />
        </div>
        <TextField
          label="Email address"
          placeholder="Enter your email address"
          value={email}
          onValueChange={setEmail}
          type="email"
        />
        <p className="text-body text-secondary">
          Or{" "}
          <button type="button" className="onboarding-text-action">
            Continue with private key
          </button>
        </p>
      </div>
      <OnboardingActions />
    </OnboardingFrame>
  );
}

function WelcomeStage() {
  const [email, setEmail] = useState("");

  return (
    <OnboardingFrame legal>
      <h2 className="text-title text-primary">Welcome back to Buzz</h2>
      <div className="onboarding-fields">
        <TextField
          label="Email address"
          placeholder="Enter your email address"
          value={email}
          onValueChange={setEmail}
          type="email"
        />
        <p className="text-body text-secondary">
          Or{" "}
          <button type="button" className="onboarding-text-action">
            Sign in with private key
          </button>
        </p>
      </div>
      <OnboardingActions />
    </OnboardingFrame>
  );
}

function CheckEmailStage() {
  return (
    <OnboardingFrame legal>
      <div className="onboarding-heading-lockup">
        <h2 className="text-title text-primary">Check your email</h2>
        <p className="text-body text-secondary">
          We sent a code to{" "}
          <strong className="text-primary font-semibold">
            name@example.com
          </strong>
        </p>
      </div>
      <div className="onboarding-code-content">
        <CodeInput
          label="Verification code"
          labelHidden
          autoFocus
          defaultValue="28"
        />
        <p className="text-body-sm text-tertiary">
          Didn’t get the code?{" "}
          <button
            type="button"
            className="onboarding-resend-action text-body-sm text-primary font-semibold"
          >
            Resend
          </button>
        </p>
      </div>
      <div className="onboarding-actions">
        <IconButton
          aria-label="Go back"
          icon={<IconArrowLeft size={20} stroke={1.7} aria-hidden="true" />}
          variant="quiet"
          size="large"
          shape="round"
        />
      </div>
    </OnboardingFrame>
  );
}

const IDENTITY_POINTS = [
  {
    label: "Stored securely on this device",
    icon: <IconShieldCheck size={20} stroke={1.7} aria-hidden="true" />,
  },
  {
    label: "Never share it—anyone with this key can sign in as you",
    icon: <IconBan size={20} stroke={1.7} aria-hidden="true" />,
  },
  {
    label: "Use a secure backup to recover your account",
    icon: <IconDeviceFloppy size={20} stroke={1.7} aria-hidden="true" />,
  },
] as const;

function IdentityStage() {
  return (
    <OnboardingFrame legal>
      <div className="onboarding-heading-lockup">
        <h2 className="text-title text-primary">
          Create a private identity key
        </h2>
        <p className="text-body-lg text-secondary">
          This key will be how you log into Buzz. You can use it across Buzz
          communities and other platforms.
        </p>
        <button
          type="button"
          className="onboarding-heading-action onboarding-text-action text-body text-primary"
        >
          Learn how identity keys work
        </button>
      </div>
      <ul className="onboarding-identity-points">
        {IDENTITY_POINTS.map((point) => (
          <li key={point.label} className="text-body text-primary">
            <span className="onboarding-point-icon">{point.icon}</span>
            {point.label}
          </li>
        ))}
      </ul>
      <OnboardingActions />
    </OnboardingFrame>
  );
}

const PROFILE_CHOICES = [
  { src: "/onboarding/profile-sun.svg", label: "Sun profile" },
  { src: "/onboarding/profile-green.svg", label: "Green profile" },
  { src: "/onboarding/profile-purple.png", label: "Purple profile" },
  { src: "/onboarding/profile-pink.png", label: "Pink profile" },
] as const;

function ProfileStage() {
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  return (
    <OnboardingFrame>
      <h2 className="text-title text-primary">Build your profile</h2>
      <fieldset className="onboarding-profile-choices">
        <legend className="sr-only">Choose a profile</legend>
        <button
          type="button"
          className="onboarding-profile-add onboarding-profile-button"
          aria-label="Add your own profile picture"
          onClick={() => setSelected("custom")}
          data-selected={selected === "custom" || undefined}
        >
          <IconPlus size={24} stroke={1.7} aria-hidden="true" />
        </button>
        {PROFILE_CHOICES.map((choice) => (
          <button
            key={choice.label}
            type="button"
            className="onboarding-profile-button onboarding-profile-choice"
            aria-label={choice.label}
            onClick={() => setSelected(choice.label)}
            data-selected={selected === choice.label || undefined}
          >
            <img src={choice.src} alt="" />
          </button>
        ))}
      </fieldset>
      <TextField
        label="Name"
        placeholder="Enter your name"
        value={name}
        onValueChange={setName}
      />
      <OnboardingActions />
    </OnboardingFrame>
  );
}

const STAGES = [
  { label: "Create a Buzz account", Component: AccountStage },
  { label: "Welcome back to Buzz", Component: WelcomeStage },
  { label: "Check your email", Component: CheckEmailStage },
  { label: "Create a private identity key", Component: IdentityStage },
  { label: "Build your profile", Component: ProfileStage },
] as const;

export function OnboardingShowcase() {
  return (
    <div className="onboarding-showcase">
      <p className="onboarding-showcase-intro text-body text-secondary">
        Five selected onboarding moments from Figma, rebuilt with the current
        Buzz backdrop, surface, type, and control system.
      </p>
      {STAGES.map(({ label, Component }) => (
        <section className="onboarding-showcase-stage" key={label}>
          <h2 className="text-body-sm text-tertiary">{label}</h2>
          <Component />
        </section>
      ))}
    </div>
  );
}
