import * as React from "react";

import {
  FirstRunButton,
  FirstRunError,
  FirstRunLink,
  FirstRunPasswordField,
} from "./firstRunControls";
import { FirstRunHead } from "./FirstRunFrame";
import {
  CheckIcon,
  ClockIcon,
  HintIcon,
  LockIcon,
  MailIcon,
} from "./firstRunIcons";
import type { PresentationProps } from "./OnboardingSceneTypes";
import type { OnboardingSceneId } from "./onboardingScenes";

type CodePurpose = "verify" | "reset";
type CodeView =
  | "request"
  | "entry"
  | "wrong"
  | "verifying"
  | "expired"
  | "locked"
  | "resent"
  | "network"
  | "change-email"
  | "password"
  | "success";

type CodeRoute = { purpose: CodePurpose; view: CodeView };
const digitLabels = [
  "Digit 1 of 6",
  "Digit 2 of 6",
  "Digit 3 of 6",
  "Digit 4 of 6",
  "Digit 5 of 6",
  "Digit 6 of 6",
] as const;

const routeMap: Partial<Record<OnboardingSceneId, CodeRoute>> = {
  forgot: { purpose: "reset", view: "request" },
  "email-sent": { purpose: "reset", view: "entry" },
  "new-password": { purpose: "reset", view: "password" },
  "reset-done": { purpose: "reset", view: "success" },
  "reset-expired": { purpose: "reset", view: "expired" },
  verify: { purpose: "verify", view: "entry" },
  "verify-error": { purpose: "verify", view: "wrong" },
  "verify-verifying": { purpose: "verify", view: "verifying" },
  "verify-expired": { purpose: "verify", view: "expired" },
  "verify-locked": { purpose: "verify", view: "locked" },
  "verify-resent": { purpose: "verify", view: "resent" },
  "verify-network": { purpose: "verify", view: "network" },
  "verify-change-email": { purpose: "verify", view: "change-email" },
  "verify-done": { purpose: "verify", view: "success" },
  "reset-verifying": { purpose: "reset", view: "verifying" },
  "reset-error": { purpose: "reset", view: "wrong" },
  "reset-locked": { purpose: "reset", view: "locked" },
  "reset-resent": { purpose: "reset", view: "resent" },
  "reset-network": { purpose: "reset", view: "network" },
  "reset-change-email": { purpose: "reset", view: "change-email" },
};

export function isEmailCodeScene(scene: OnboardingSceneId): boolean {
  return routeMap[scene] !== undefined;
}

/** Where a reset scene sits in the four reset steps (Email, Code, New password, Done). */
export function resetStepIndex(scene: OnboardingSceneId): number | null {
  const route = routeMap[scene];
  if (route?.purpose !== "reset") return null;
  if (route.view === "request" || route.view === "change-email") return 0;
  if (route.view === "password") return 2;
  if (route.view === "success") return 3;
  return 1;
}

function fixtureCode(scene: OnboardingSceneId, visualOnly?: boolean) {
  if (!visualOnly) return "";
  return scene.endsWith("-verifying") ? "123456" : "";
}

function fixtureCountdown(view: CodeView, visualOnly?: boolean) {
  if (!visualOnly) return 0;
  if (view === "locked") return 60;
  if (
    view === "entry" ||
    view === "wrong" ||
    view === "resent" ||
    view === "network" ||
    view === "verifying"
  )
    return 24;
  return 0;
}

function useCountdown(seconds: number) {
  const [remaining, setRemaining] = React.useState(seconds);
  React.useEffect(() => {
    setRemaining(seconds);
    if (seconds <= 0) return;
    const endsAt = Date.now() + seconds * 1000;
    const timer = window.setInterval(() => {
      setRemaining(Math.max(0, Math.ceil((endsAt - Date.now()) / 1000)));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [seconds]);
  return remaining;
}

function initialViewCode(view: CodeView, visualOnly: boolean) {
  return view === "network" || view === "verifying"
    ? fixtureCode(
        view === "network" ? "verify-network" : "verify-verifying",
        visualOnly,
      )
    : "";
}

function CodeFields({
  code,
  disabled,
  invalid,
  purpose,
  onChange,
}: {
  code: string;
  disabled: boolean;
  invalid: boolean;
  purpose: CodePurpose;
  onChange: (value: string) => void;
}) {
  const fields = React.useRef<Array<HTMLInputElement | null>>([]);
  const groupLabel =
    purpose === "reset"
      ? "Six-digit password reset code"
      : "Six-digit email verification code";
  const setDigit = (index: number, value: string) => {
    const next = code.padEnd(6, " ").split("");
    next[index] = value.slice(-1);
    onChange(next.join("").replace(/\s/g, ""));
  };

  return (
    <fieldset className="c3fr-otp" disabled={disabled}>
      <legend className="c3fr-sr-only">{groupLabel}</legend>
      {digitLabels.map((label, index) => (
        <input
          aria-describedby="otp-entry-help"
          aria-invalid={invalid || undefined}
          aria-label={label}
          autoComplete={index === 0 ? "one-time-code" : "off"}
          inputMode="numeric"
          key={label}
          maxLength={1}
          onChange={(event) => {
            const digits = event.currentTarget.value.replace(/\D/g, "");
            if (digits.length > 1) {
              onChange(digits.slice(0, 6));
              fields.current[Math.min(digits.length, 5)]?.focus();
              return;
            }
            setDigit(index, digits);
            if (digits && index < 5) fields.current[index + 1]?.focus();
          }}
          onKeyDown={(event) => {
            if (event.key === "Backspace" && !code[index] && index > 0) {
              event.preventDefault();
              setDigit(index - 1, "");
              fields.current[index - 1]?.focus();
            } else if (
              event.key === "ArrowLeft" ||
              event.key === "ArrowRight"
            ) {
              event.preventDefault();
              const direction = event.key === "ArrowLeft" ? -1 : 1;
              fields.current[
                Math.max(0, Math.min(5, index + direction))
              ]?.focus();
            }
          }}
          onPaste={(event) => {
            const digits = event.clipboardData
              .getData("text")
              .replace(/\D/g, "")
              .slice(0, 6);
            if (!digits) return;
            event.preventDefault();
            onChange(digits);
            fields.current[Math.min(digits.length, 5)]?.focus();
          }}
          pattern="[0-9]"
          ref={(node) => {
            fields.current[index] = node;
          }}
          type="text"
          value={code[index] ?? ""}
        />
      ))}
    </fieldset>
  );
}

/** What went wrong (or is happening) with the code, as a quiet notice. */
type CodeNotice = {
  tone: "error" | "sent" | "pending";
  title: string;
  text: string;
};

function codeNotice(
  view: CodeView,
  attempts: number | undefined,
  remaining: number,
): CodeNotice | null {
  switch (view) {
    case "wrong":
      return {
        tone: "error",
        title: "That code isn’t right.",
        text:
          attempts === undefined
            ? "Check the six digits and try again."
            : `${attempts} ${attempts === 1 ? "attempt" : "attempts"} left. Check the six digits and try again.`,
      };
    case "expired":
      return {
        tone: "error",
        title: "This code has expired.",
        text: "Resend a code to continue. Your email is kept.",
      };
    case "locked":
      return {
        tone: "error",
        title: "Too many attempts.",
        text:
          remaining > 0
            ? `Try again in ${remaining}s. You can resend a code after the wait.`
            : "The wait is over. Resend a fresh code to continue.",
      };
    case "network":
      return {
        tone: "error",
        title: "Couldn’t verify your code.",
        text: "Check your connection and try again. No attempt was used.",
      };
    case "resent":
      return {
        tone: "sent",
        title: "A new code is on its way.",
        text: "Use the latest email. Your previous code is no longer valid.",
      };
    case "verifying":
      return { tone: "pending", title: "Verifying your code…", text: "" };
    default:
      return null;
  }
}

/** One reset step as the card the reset board draws. */
function ResetCard({
  badge,
  step,
  title,
  lede,
  headingRef,
  children,
  tone,
}: {
  badge: React.ReactNode;
  step: number;
  title: string;
  lede: React.ReactNode;
  headingRef?: React.Ref<HTMLHeadingElement>;
  children?: React.ReactNode;
  tone?: "ok";
}) {
  return (
    <div className="c3fr-reset-card" data-tone={tone}>
      <span aria-hidden="true" className="c3fr-reset-badge" data-tone={tone}>
        {badge}
      </span>
      <div className="c3fr-head">
        <span className="c3fr-eyebrow c3fr-eyebrow-muted">
          Step {step} of 4
        </span>
        <h2
          className="c3fr-title c3fr-title-card"
          id="account-auth-title"
          ref={headingRef}
          tabIndex={-1}
        >
          {title}
        </h2>
        <p className="c3fr-lede c3fr-lede-card">{lede}</p>
      </div>
      {children}
    </div>
  );
}

function EmailCodeBody(props: PresentationProps) {
  const route = routeMap[props.scene] ?? { purpose: "verify", view: "entry" };
  const { purpose, view } = route;
  const isReset = purpose === "reset";
  const code =
    props.authCode?.value ?? initialViewCode(view, !!props.data.visualOnly);
  const [localCode, setLocalCode] = React.useState(code);
  const codeValue = props.authCode?.value ?? localCode;
  const onCodeChange = props.authCode?.onCodeChange ?? setLocalCode;
  const fixtureRemaining = useCountdown(
    fixtureCountdown(view, props.data.visualOnly),
  );
  const remaining = props.authCode
    ? (props.authCode.cooldownSecs ?? 0)
    : fixtureRemaining;
  const pending =
    props.authCode?.pending ??
    props.pending ??
    (props.data.visualOnly && view === "verifying");
  const locked = view === "locked";
  const expired = view === "expired";
  const disabled = pending || locked || expired;
  const attempts =
    props.authCode?.attemptsLeft ??
    (props.data.visualOnly && view === "wrong" ? 2 : undefined);
  const emailValue = props.data.visualOnly
    ? props.data.email || "you@example.com"
    : props.data.email;
  const [localEmail, setLocalEmail] = React.useState(emailValue);
  const [password, setPassword] = React.useState("");
  const [confirmation, setConfirmation] = React.useState("");
  const email = props.data.visualOnly ? emailValue : localEmail;

  const setEmail = (value: string) => {
    setLocalEmail(value);
    props.onEmailChange?.(value);
  };
  const routeSubmit =
    view === "request" || view === "change-email"
      ? props.onSubmit
      : view === "password"
        ? (props.authCode?.onPasswordSubmit ?? props.onSubmit)
        : (props.authCode?.onCodeSubmit ?? props.onSubmit);
  const error = props.authCode?.error || props.error;
  const changeEmail =
    props.authCode?.onChangeEmail ??
    (() =>
      props.onNavigate?.(
        isReset ? "reset-change-email" : "verify-change-email",
      ));

  if (view === "request" || view === "change-email") {
    const changing = view === "change-email";
    if (!isReset) {
      return (
        <form
          className="c3fr-form"
          data-otp-state={view}
          id="email-code-form"
          onSubmit={routeSubmit}
        >
          <FirstRunHead
            headingRef={props.headingRef}
            lede="We’ll send a new code to this address. The previous code won’t be used here."
            title="Change email."
          />
          <div className="c3fr-field-group">
            <label className="c3fr-label" htmlFor="otp-email">
              Email address
            </label>
            <input
              autoComplete="email"
              className="c3fr-input"
              id="otp-email"
              onChange={(event) => setEmail(event.currentTarget.value)}
              required
              type="email"
              value={email}
            />
          </div>
          {error ? <FirstRunError>{error}</FirstRunError> : null}
          <FirstRunButton disabled={pending} kind="primary" type="submit">
            {pending ? "Sending…" : "Send code"}
          </FirstRunButton>
        </form>
      );
    }
    return (
      <form data-otp-state={view} id="email-code-form" onSubmit={routeSubmit}>
        <ResetCard
          badge={<MailIcon size={26} />}
          headingRef={props.headingRef}
          lede={
            changing
              ? "We’ll send a new code to this address. The previous code won’t be used here."
              : "We’ll email you a six-digit code to set a new one."
          }
          step={1}
          title={changing ? "Change email." : "Reset your password."}
        >
          <div className="c3fr-field-group">
            <label className="c3fr-label" htmlFor="otp-email">
              Email address
            </label>
            <input
              autoComplete="email"
              className="c3fr-input"
              id="otp-email"
              onChange={(event) => setEmail(event.currentTarget.value)}
              required
              type="email"
              value={email}
            />
          </div>
          {error ? <FirstRunError>{error}</FirstRunError> : null}
          <FirstRunButton disabled={pending} kind="primary" type="submit">
            {pending ? "Sending…" : "Send code"}
          </FirstRunButton>
        </ResetCard>
      </form>
    );
  }

  if (view === "password") {
    const length = props.data.visualOnly ? 8 : 10;
    return (
      <form data-otp-state={view} id="email-code-form" onSubmit={routeSubmit}>
        <ResetCard
          badge={<LockIcon size={26} />}
          headingRef={props.headingRef}
          lede={
            <>
              Enter a new password for <strong>{email}</strong>.
            </>
          }
          step={3}
          title="A fresh password."
        >
          <FirstRunPasswordField
            ariaLabelName="new password"
            autoComplete="new-password"
            id="otp-password"
            label="New password"
            minLength={length}
            onChange={(event) => {
              const value = event.currentTarget.value;
              if (props.authCode?.onNewPasswordChange) {
                props.authCode.onNewPasswordChange(value);
              } else {
                setPassword(value);
              }
            }}
            value={props.authCode?.newPassword ?? password}
          />
          <div className="c3fr-field-group">
            <FirstRunPasswordField
              ariaLabelName="confirmed password"
              autoComplete="new-password"
              id="otp-confirm"
              label="Confirm password"
              minLength={length}
              onChange={(event) => {
                const value = event.currentTarget.value;
                if (props.authCode?.onConfirmPasswordChange) {
                  props.authCode.onConfirmPasswordChange(value);
                } else {
                  setConfirmation(value);
                }
              }}
              value={props.authCode?.confirmPassword ?? confirmation}
            />
            <span className="c3fr-hint">
              <HintIcon size={15} strokeWidth={2.2} />
              {props.data.visualOnly
                ? "Use at least 8 characters in this preview."
                : "Use at least 10 characters."}
            </span>
          </div>
          {error ? <FirstRunError>{error}</FirstRunError> : null}
          <FirstRunButton disabled={pending} kind="primary" type="submit">
            {pending ? "Updating…" : "Set new password"}
          </FirstRunButton>
        </ResetCard>
      </form>
    );
  }

  if (view === "success") {
    if (isReset) {
      return (
        <ResetCard
          badge={<CheckIcon size={26} strokeWidth={3} />}
          headingRef={props.headingRef}
          lede="You’re all set. Your password has been updated, and your Colony is right where you left it."
          step={4}
          title="Password updated."
          tone="ok"
        >
          <FirstRunButton
            kind="primary"
            onClick={() => props.onNavigate?.("signin")}
          >
            Back to sign in
          </FirstRunButton>
        </ResetCard>
      );
    }
    return (
      <div className="c3fr-form" data-otp-state={view}>
        <span aria-hidden="true" className="c3fr-badge" data-tone="ok">
          <CheckIcon size={28} strokeWidth={3} />
        </span>
        <FirstRunHead
          headingRef={props.headingRef}
          lede="You’re ready to set up your business."
          title="Email verified."
        />
        <FirstRunButton
          kind="primary"
          onClick={() => props.onNavigate?.("business")}
        >
          Continue to setup
        </FirstRunButton>
      </div>
    );
  }

  const isEntry =
    view === "entry" ||
    view === "wrong" ||
    view === "expired" ||
    view === "locked" ||
    view === "resent" ||
    view === "network" ||
    view === "verifying";
  if (!isEntry) return null;
  const notice = codeNotice(view, attempts, remaining);
  const codeLabel = isReset ? "password reset" : "email verification";
  const resend = (
    <>
      {remaining > 0 ? (
        <button className="c3fr-resend-wait" disabled type="button">
          <ClockIcon size={15} strokeWidth={2.2} />
          <span>Resend code in {remaining}s</span>
        </button>
      ) : (
        <FirstRunLink
          disabled={pending}
          onClick={props.authCode?.onResend ?? props.onResend}
        >
          Resend code
        </FirstRunLink>
      )}
    </>
  );
  const noticeBlock = notice ? (
    <div
      className="c3fr-notice"
      data-tone={notice.tone}
      role={notice.tone === "error" ? "alert" : "status"}
    >
      {notice.tone === "pending" ? (
        <span aria-hidden="true" className="c3fr-spinner" />
      ) : null}
      <strong>{notice.title}</strong>
      {notice.text ? <span>{notice.text}</span> : null}
    </div>
  ) : null;
  const alertLine = notice ? null : error ? (
    <FirstRunError>{error}</FirstRunError>
  ) : null;

  if (isReset) {
    return (
      <form data-otp-state={view} id="email-code-form" onSubmit={routeSubmit}>
        <ResetCard
          badge={<MailIcon size={26} />}
          headingRef={props.headingRef}
          lede={
            <>
              If an account exists for this address, we’ve sent a six-digit
              reset code to <strong>{email}</strong>.
            </>
          }
          step={2}
          title="Check your email."
        >
          {noticeBlock}
          <CodeFields
            code={codeValue}
            disabled={disabled}
            invalid={view === "wrong"}
            onChange={onCodeChange}
            purpose={purpose}
          />
          <p className="c3fr-help" id="otp-entry-help">
            You can paste the whole code.
          </p>
          {alertLine}
          <div className="c3fr-reset-links">
            {resend}
            <FirstRunLink disabled={pending} onClick={changeEmail}>
              Change email
            </FirstRunLink>
          </div>
          <FirstRunButton
            data-testid="otp-continue"
            disabled={pending || disabled || codeValue.length !== 6}
            kind="primary"
            type="submit"
          >
            {pending ? "Checking…" : "Continue"}
          </FirstRunButton>
          <span className="c3fr-sr-only">Code purpose: {codeLabel}</span>
        </ResetCard>
      </form>
    );
  }

  return (
    <form
      className="c3fr-form"
      data-otp-state={view}
      id="email-code-form"
      onSubmit={routeSubmit}
      style={{ "--c3fr-gap": 26 } as React.CSSProperties}
    >
      <span aria-hidden="true" className="c3fr-badge">
        <MailIcon size={28} />
      </span>
      <FirstRunHead
        headingRef={props.headingRef}
        lede={
          <>
            Enter the six-digit code we sent to <strong>{email}</strong>.
          </>
        }
        title="Check your email."
      />
      {noticeBlock}
      <div className="c3fr-field-group c3fr-code-group">
        <span className="c3fr-label c3fr-code-legend">Verification code</span>
        <CodeFields
          code={codeValue}
          disabled={disabled}
          invalid={view === "wrong"}
          onChange={onCodeChange}
          purpose={purpose}
        />
        <div className="c3fr-resend" id="otp-entry-help">
          {resend}
          <span className="c3fr-sr-only">You can paste the whole code.</span>
        </div>
      </div>
      {alertLine}
      <FirstRunButton
        data-testid="otp-continue"
        disabled={pending || disabled || codeValue.length !== 6}
        kind="primary"
        type="submit"
      >
        {pending ? "Verifying…" : "Continue"}
      </FirstRunButton>
      <p className="c3fr-note">
        Can’t find it? Check your spam folder, or wait for the timer and send a
        fresh code.
      </p>
      <span className="c3fr-sr-only">Code purpose: {codeLabel}</span>
    </form>
  );
}

export function EmailCodePresentation(props: PresentationProps) {
  return <EmailCodeBody {...props} />;
}
