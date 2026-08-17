import * as React from "react";

type InviteJoinPolicy = {
  terms_markdown?: string;
  privacy_markdown?: string;
  age_attestation_required: boolean;
};

type PolicyCheckboxProps = {
  accessibleLabel: string;
  checked: boolean;
  children: React.ReactNode;
  onCheckedChange: (checked: boolean) => void;
};

function PolicyCheckbox({
  accessibleLabel,
  checked,
  children,
  onCheckedChange,
}: PolicyCheckboxProps) {
  const id = React.useId();

  return (
    <div className="relative">
      <input
        aria-label={accessibleLabel}
        checked={checked}
        className="peer sr-only"
        id={id}
        onChange={(event) => onCheckedChange(event.target.checked)}
        type="checkbox"
      />
      <label
        className="flex cursor-pointer items-start gap-3 rounded-sm text-left text-xs leading-5 text-black/60 peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-black"
        htmlFor={id}
      >
        <span
          aria-hidden="true"
          className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-[background-color,border-color] duration-150 motion-reduce:transition-none ${
            checked ? "border-black bg-black" : "border-black/40 bg-white"
          }`}
        >
          <svg
            aria-hidden="true"
            className="h-3 w-3 text-white"
            fill="none"
            viewBox="0 0 16 16"
          >
            <path
              className={
                checked
                  ? "transition-[stroke-dashoffset] duration-[180ms] [transition-timing-function:cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
                  : "transition-none"
              }
              d="M3 8.25 6.25 11.5 13 4.75"
              pathLength="16"
              stroke="currentColor"
              strokeDasharray="16"
              strokeDashoffset={checked ? 0 : 16}
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth="2"
            />
          </svg>
        </span>
        <span>{children}</span>
      </label>
    </div>
  );
}

/** The invite page's age and legal confirmations. */
export function InviteJoinPolicyNotice({
  ageConfirmed,
  agreementConfirmed,
  onAgeConfirmedChange,
  onAgreementConfirmedChange,
  onShowDocument,
  policy,
}: {
  ageConfirmed: boolean;
  agreementConfirmed: boolean;
  onAgeConfirmedChange: (checked: boolean) => void;
  onAgreementConfirmedChange: (checked: boolean) => void;
  onShowDocument: (title: string, markdown: string) => void;
  policy: InviteJoinPolicy;
}) {
  const stopLabelActivation = (
    event: React.MouseEvent<HTMLButtonElement>,
    title: string,
    markdown: string,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    onShowDocument(title, markdown);
  };

  return (
    <div
      className="w-full space-y-3 rounded-xl border border-black/10 bg-black/[0.03] p-4 text-left"
      data-testid="invite-join-policy-notice"
    >
      {policy.age_attestation_required ? (
        <PolicyCheckbox
          accessibleLabel="I am 18 years of age or older."
          checked={ageConfirmed}
          onCheckedChange={onAgeConfirmedChange}
        >
          I am 18 years of age or older.
        </PolicyCheckbox>
      ) : null}

      {policy.terms_markdown || policy.privacy_markdown ? (
        <PolicyCheckbox
          accessibleLabel="I agree to the Buzz Terms of Service and Privacy Policy."
          checked={agreementConfirmed}
          onCheckedChange={onAgreementConfirmedChange}
        >
          I agree to the Buzz{" "}
          {policy.terms_markdown ? (
            <button
              className="text-black no-underline underline-offset-4 hover:text-black/70 hover:underline focus-visible:outline-1 focus-visible:outline-offset-2 focus-visible:outline-black"
              type="button"
              onClick={(event) =>
                stopLabelActivation(
                  event,
                  "Terms of Service",
                  policy.terms_markdown ?? "",
                )
              }
            >
              Terms of Service
            </button>
          ) : (
            "Terms of Service"
          )}{" "}
          and{" "}
          {policy.privacy_markdown ? (
            <button
              className="text-black no-underline underline-offset-4 hover:text-black/70 hover:underline focus-visible:outline-1 focus-visible:outline-offset-2 focus-visible:outline-black"
              type="button"
              onClick={(event) =>
                stopLabelActivation(
                  event,
                  "Privacy Policy",
                  policy.privacy_markdown ?? "",
                )
              }
            >
              Privacy Policy
            </button>
          ) : (
            "Privacy Policy"
          )}
          .
        </PolicyCheckbox>
      ) : null}
    </div>
  );
}
