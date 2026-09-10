import { Check } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { OnboardingPreviewInput } from "./OnboardingPreviewInput";
import { useOnboardingPreviewCardLayout } from "./OnboardingPreviewShell";
import { ONBOARDING_PREVIEW_CARD_INPUT_CLASS } from "./onboardingPreviewCardStyles";

type PreviewPasswordStrength = "empty" | "weak" | "moderate" | "strong";

const strengthLabelPhraseVariants = {
  animate: { transition: { staggerChildren: 0.008 } },
  exit: { transition: { staggerChildren: 0.005 } },
  initial: {},
};

const strengthLabelCharacterVariants = {
  animate: {
    filter: "blur(0)",
    opacity: 1,
    transition: { duration: 0.24, ease: [0.22, 1, 0.36, 1] as const },
    y: 0,
  },
  exit: {
    filter: "blur(0.25rem)",
    opacity: 0,
    transition: { duration: 0.16, ease: [0.64, 0, 0.78, 0] as const },
    y: "-0.5rem",
  },
  initial: { filter: "blur(0.25rem)", opacity: 0, y: "0.5rem" },
};

function previewPasswordStrength(password: string): PreviewPasswordStrength {
  if (password.length === 0) return "empty";
  if (password.length < 8) return "weak";
  if (password.length < 15) return "moderate";
  return "strong";
}

function PasswordStrengthMeter({
  descriptionId,
  password,
}: {
  descriptionId: string;
  password: string;
}) {
  const strength = previewPasswordStrength(password);
  const reduceMotion = useReducedMotion() ?? false;
  const activeCount =
    strength === "strong"
      ? 3
      : strength === "moderate"
        ? 2
        : strength === "weak"
          ? 1
          : 0;
  const label =
    strength === "empty"
      ? "Empty"
      : strength === "weak"
        ? "Weak"
        : strength === "moderate"
          ? "Moderate"
          : "Strong";
  const labelCharacters = React.useMemo(() => {
    const occurrences = new Map<string, number>();
    return [...label].map((character) => {
      const occurrence = occurrences.get(character) ?? 0;
      occurrences.set(character, occurrence + 1);
      return { character, key: `${character}-${occurrence}` };
    });
  }, [label]);

  return (
    <div
      className={cn(
        "pointer-events-none absolute right-4 top-1/2 flex -translate-y-1/2 items-center gap-2 text-xs font-semibold transition-colors duration-150 motion-reduce:transition-none",
        strength === "weak" && "text-red-700",
        strength === "moderate" && "text-amber-700",
        strength === "strong" && "text-green-700",
        strength === "empty" && "text-transparent",
      )}
      data-strength={strength}
      data-testid="onboarding-preview-password-strength"
    >
      <span className="relative flex h-5 w-[4.75rem] items-center justify-end overflow-visible text-right leading-none">
        {reduceMotion ? (
          <span>{strength === "empty" ? "" : label}</span>
        ) : (
          <AnimatePresence initial={false} mode="wait">
            {strength === "empty" ? null : (
              <motion.span
                animate="animate"
                className="absolute inset-x-0 top-1/2 -translate-y-1/2 whitespace-nowrap"
                exit="exit"
                initial="initial"
                key={label}
                variants={strengthLabelPhraseVariants}
              >
                {labelCharacters.map(({ character, key }) => (
                  <motion.span
                    className="inline-block will-change-[transform,opacity,filter]"
                    key={`${label}-${key}`}
                    variants={strengthLabelCharacterVariants}
                  >
                    {character}
                  </motion.span>
                ))}
              </motion.span>
            )}
          </AnimatePresence>
        )}
      </span>
      <span
        aria-hidden
        className={cn(
          "relative flex h-8 w-5 flex-col items-center justify-center gap-0.5",
          strength === "empty" && "opacity-0",
        )}
        data-testid="onboarding-preview-password-strength-indicator"
      >
        {[3, 2, 1].map((level) => {
          const isTopDot = level === 3;
          const positionClass =
            level === 3
              ? strength === "strong"
                ? "top-1/2 -translate-y-1/2"
                : "top-1"
              : level === 2
                ? "top-[13px]"
                : "top-[22px]";
          return (
            <span
              className={cn(
                "absolute left-1/2 flex -translate-x-1/2 items-center justify-center overflow-hidden transition-[top,width,height,border-radius,background-color,opacity,transform] duration-150 ease-in-out motion-reduce:transition-none",
                positionClass,
                isTopDot && strength === "strong"
                  ? "h-5 w-5 rounded-full bg-green-600"
                  : "h-1.5 w-1.5 rounded-full",
                !isTopDot && strength === "strong"
                  ? level === 2
                    ? "translate-y-1 scale-0 opacity-0"
                    : "-translate-y-1.5 scale-0 opacity-0"
                  : "scale-100 opacity-100",
                strength !== "strong" &&
                  strength !== "empty" &&
                  activeCount >= level &&
                  "bg-amber-500",
                strength === "weak" && activeCount >= level && "bg-red-500",
                strength !== "strong" &&
                  activeCount < level &&
                  "bg-foreground/20",
              )}
              data-level={level}
              key={level}
            >
              {isTopDot ? (
                <Check
                  className={cn(
                    "h-3.5 w-3.5 text-white transition-[opacity,transform] duration-120 ease-in-out motion-reduce:transition-none",
                    strength === "strong"
                      ? "scale-100 opacity-100"
                      : "scale-75 opacity-0",
                  )}
                  data-testid="onboarding-preview-password-strength-check"
                />
              ) : null}
            </span>
          );
        })}
      </span>
      <span
        aria-atomic="true"
        aria-live="polite"
        className="sr-only"
        id={descriptionId}
        role="status"
      >
        Password strength: {label}
      </span>
    </div>
  );
}

export function PreviewCredentialsFields({
  email,
  emailId,
  onEmailChange,
  onPasswordChange,
  onConfirmPasswordChange,
  confirmPassword,
  password,
  passwordAutoComplete,
  passwordHelp,
  passwordId,
  passwordPlaceholder,
  showPasswordStrength = false,
}: {
  email: string;
  emailId: string;
  onEmailChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onConfirmPasswordChange?: (value: string) => void;
  confirmPassword?: string;
  password: string;
  passwordAutoComplete: "current-password" | "new-password";
  passwordHelp?: React.ReactNode;
  passwordId: string;
  passwordPlaceholder: string;
  showPasswordStrength?: boolean;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const passwordStrengthDescriptionId = React.useId();

  return (
    <>
      <div>
        <label
          className="mb-2 block text-sm font-medium text-foreground"
          htmlFor={emailId}
        >
          Email
        </label>
        <OnboardingPreviewInput
          autoComplete="email"
          className={
            cardLayout
              ? ONBOARDING_PREVIEW_CARD_INPUT_CLASS
              : "h-12 rounded-2xl border-foreground/15 bg-white px-4"
          }
          id={emailId}
          onChange={(event) => onEmailChange(event.target.value)}
          placeholder="Enter your email address"
          smooth={cardLayout}
          type="email"
          value={email}
        />
      </div>
      <div>
        <div className="mb-2 flex items-center justify-between gap-3">
          <label
            className="block text-sm font-medium text-foreground"
            htmlFor={passwordId}
          >
            Password
          </label>
          {passwordHelp ? (
            <div className="shrink-0 text-right">{passwordHelp}</div>
          ) : null}
        </div>
        <div className="relative">
          <OnboardingPreviewInput
            aria-describedby={
              showPasswordStrength ? passwordStrengthDescriptionId : undefined
            }
            autoComplete={passwordAutoComplete}
            className={cn(
              cardLayout
                ? ONBOARDING_PREVIEW_CARD_INPUT_CLASS
                : "h-12 rounded-2xl border-foreground/15 bg-white px-4",
              showPasswordStrength ? "pr-28" : "pr-4",
            )}
            id={passwordId}
            onChange={(event) => onPasswordChange(event.target.value)}
            placeholder={passwordPlaceholder}
            smooth={cardLayout}
            type="password"
            value={password}
          />
          {showPasswordStrength ? (
            <PasswordStrengthMeter
              descriptionId={passwordStrengthDescriptionId}
              password={password}
            />
          ) : null}
        </div>
        {onConfirmPasswordChange ? (
          <div className="mt-5">
            <label
              className="mb-2 block text-sm font-medium text-foreground"
              htmlFor={`${passwordId}-confirmation`}
            >
              Confirm password
            </label>
            <OnboardingPreviewInput
              autoComplete="new-password"
              className={
                cardLayout
                  ? ONBOARDING_PREVIEW_CARD_INPUT_CLASS
                  : "h-12 rounded-2xl border-foreground/15 bg-white px-4"
              }
              id={`${passwordId}-confirmation`}
              onChange={(event) => onConfirmPasswordChange(event.target.value)}
              placeholder="Confirm your password"
              smooth={cardLayout}
              type="password"
              value={confirmPassword ?? ""}
            />
          </div>
        ) : null}
      </div>
    </>
  );
}
