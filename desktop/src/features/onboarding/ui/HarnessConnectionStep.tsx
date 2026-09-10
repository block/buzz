import { openUrl } from "@tauri-apps/plugin-opener";
import { ChevronRight, CreditCard, ExternalLink, KeyRound } from "lucide-react";
import * as React from "react";

import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import {
  getProviderApiKeyLabel,
  getPersonaModelOptions,
  PERSONA_LLM_PROVIDER_OPTIONS,
  runtimeSupportsLlmProviderSelection,
} from "@/features/agents/ui/agentConfigOptions";
import {
  BUZZ_AGENT_THINKING_EFFORT_VALUES,
  getProviderEffortConfig,
} from "@/features/agents/ui/buzzAgentConfig";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import { OnboardingPreviewInput } from "./OnboardingPreviewInput";
import {
  OnboardingPreviewStep,
  useOnboardingPreviewCardLayout,
} from "./OnboardingPreviewShell";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
import { getRuntimeDisplayLabel, RuntimeIcon } from "./RuntimeIcon";
import { HARNESS_CONNECTION_OPTIONS } from "./harnessConnectionOptions";
import { ONBOARDING_PREVIEW_CARD_INPUT_CLASS } from "./onboardingPreviewCardStyles";

export type HarnessConnectionMethod = "subscription" | "api";

export type HarnessConnectionOption = {
  methods: readonly HarnessConnectionMethod[];
  runtime: AcpRuntimeCatalogEntry;
};

export { HARNESS_CONNECTION_OPTIONS };

function HarnessPreviewStep({
  allowWideContent = false,
  children,
  embedded = false,
  onBack,
  testId,
  total,
}: {
  allowWideContent?: boolean;
  children: React.ReactNode;
  embedded?: boolean;
  onBack: () => void;
  testId: string;
  total: number;
}) {
  if (embedded) {
    return (
      <div className="flex min-h-0 flex-1 flex-col" data-testid={testId}>
        {children}
      </div>
    );
  }

  return (
    <OnboardingPreviewStep
      allowWideContent={allowWideContent}
      current={3}
      onBack={onBack}
      testId={testId}
      total={total}
    >
      {children}
    </OnboardingPreviewStep>
  );
}

const CONNECTION_METHOD_OPTIONS = [
  { label: "Subscription", value: "subscription" },
  { label: "API", value: "api" },
] as const;

/**
 * Reusable row list for selecting or installing an AI client. The caller owns
 * availability and side effects so onboarding can remain a session-only
 * workshop while an in-app surface can wire the same presentation to Tauri.
 */
export function HarnessConnectionList({
  installedIds,
  method,
  onSelect,
  options,
}: {
  installedIds: ReadonlySet<string>;
  method?: HarnessConnectionMethod;
  onSelect: (option: HarnessConnectionOption) => void;
  options: readonly HarnessConnectionOption[];
}) {
  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  const [canScrollUp, setCanScrollUp] = React.useState(false);
  const [canScrollDown, setCanScrollDown] = React.useState(false);
  const orderedOptions = React.useMemo(() => {
    const group = ({ runtime }: HarnessConnectionOption) => {
      if (runtime.id === "buzz-agent") return 0;
      if (runtime.id === "goose") return 1;
      return installedIds.has(runtime.id) ? 2 : 3;
    };
    return [...options].sort((left, right) => group(left) - group(right));
  }, [installedIds, options]);
  const updateScrollEdges = React.useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    setCanScrollUp(element.scrollTop > 1);
    setCanScrollDown(
      element.scrollTop + element.clientHeight < element.scrollHeight - 1,
    );
  }, []);

  React.useEffect(() => {
    updateScrollEdges();
    const element = scrollRef.current;
    if (!element) return;
    const observer = new ResizeObserver(updateScrollEdges);
    observer.observe(element);
    return () => observer.disconnect();
  }, [updateScrollEdges]);

  return (
    <div className="relative -mx-2 min-h-0 w-[calc(100%+1rem)] flex-1">
      <div
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-x-0 top-0 z-10 h-4 transition-opacity duration-150 motion-reduce:transition-none",
          canScrollUp ? "opacity-100" : "opacity-0",
        )}
        style={{
          background: "linear-gradient(to bottom, white, rgb(255 255 255 / 0))",
        }}
      />
      <div
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-x-0 bottom-0 z-10 h-4 transition-opacity duration-150 motion-reduce:transition-none",
          canScrollDown ? "opacity-100" : "opacity-0",
        )}
        style={{
          background: "linear-gradient(to top, white, rgb(255 255 255 / 0))",
        }}
      />
      <div
        className="h-full min-h-0 space-y-1 overflow-y-auto overscroll-contain pr-1"
        data-testid="onboarding-preview-harness-list"
        onScroll={updateScrollEdges}
        ref={scrollRef}
      >
        {orderedOptions.map((option, index) => {
          const { runtime } = option;
          const installed = installedIds.has(runtime.id);
          const previousOption = orderedOptions[index - 1];
          const startsNotInstalledSection =
            !installed &&
            (previousOption === undefined ||
              installedIds.has(previousOption.runtime.id));
          const label = getRuntimeDisplayLabel(runtime);
          const isRecommendedBuzzOption =
            method === "api" && runtime.id === "buzz-agent";
          const rowClassName =
            "group flex min-h-14 w-full items-center gap-3 rounded-xl px-2 py-2 text-left text-sm font-medium text-foreground transition-colors duration-150 ease-out hover:bg-foreground/[0.04] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/20 motion-reduce:transition-none";
          const contents = (
            <>
              <span className="flex size-10 shrink-0 items-center justify-start">
                <RuntimeIcon className="size-9" runtime={runtime} />
              </span>
              <span className="flex min-w-0 flex-1 items-center gap-2">
                <span className="truncate">{label}</span>
                {isRecommendedBuzzOption ? (
                  <>
                    <span
                      aria-hidden="true"
                      className="shrink-0 text-muted-foreground/50"
                    >
                      ·
                    </span>
                    <span className="inline-flex shrink-0 items-center rounded-md bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
                      Recommended
                    </span>
                  </>
                ) : null}
              </span>
            </>
          );

          return (
            <React.Fragment key={runtime.id}>
              {startsNotInstalledSection ? (
                <p
                  className="px-2 pb-1 pt-4 text-xs font-medium text-muted-foreground"
                  data-testid="onboarding-preview-harness-not-installed"
                >
                  Not installed
                </p>
              ) : null}
              <button
                className={rowClassName}
                data-testid={`onboarding-preview-harness-${runtime.id}`}
                onClick={() => onSelect(option)}
                type="button"
              >
                {contents}
                <span className="flex size-10 shrink-0 items-center justify-end">
                  <ChevronRight
                    aria-hidden="true"
                    className="size-4 text-muted-foreground transition-colors duration-150 ease-out group-hover:text-foreground motion-reduce:transition-none"
                  />
                </span>
              </button>
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
}

export function HarnessConnectionPreview({
  embedded = false,
  installedIds,
  method,
  onBack,
  onHelp,
  onSelect,
  total,
}: {
  embedded?: boolean;
  installedIds: ReadonlySet<string>;
  method?: HarnessConnectionMethod;
  onBack: () => void;
  onHelp?: () => void;
  onSelect: (option: HarnessConnectionOption) => void;
  total: number;
}) {
  const options = method
    ? HARNESS_CONNECTION_OPTIONS.filter(({ methods }) =>
        methods.includes(method),
      )
    : HARNESS_CONNECTION_OPTIONS;

  return (
    <HarnessPreviewStep
      embedded={embedded}
      onBack={onBack}
      testId="onboarding-preview-harness-connection"
      total={total}
    >
      <OnboardingSlideTransition
        className="flex min-h-0 flex-1 flex-col"
        containerClassName="min-h-0 flex-1"
        transitionKey="preview-harness-connection"
      >
        <div className="shrink-0">
          <h1 className="text-title font-normal text-foreground">
            {method === "subscription"
              ? "Continue with an AI subscription"
              : method === "api"
                ? "Choose a harness"
                : "Connect your AI client"}
          </h1>
          {method === "subscription" ? (
            <p className="mt-2 text-base leading-6 text-foreground/80">
              Subscriptions connect through a compatible harness, like Claude
              Code or Codex. Choose yours to sign in.
            </p>
          ) : method === "api" ? (
            <p className="mt-2 text-base leading-6 text-foreground/80">
              Choose how your agents will connect to AI providers. You can
              change this at any time.
            </p>
          ) : (
            <p className="mt-2 text-base leading-6 text-foreground/80">
              Choose which AI client your agents will use. You can change this
              anytime.{" "}
              {onHelp ? (
                <Button
                  className="inline-flex h-auto p-0 align-baseline text-base leading-6 text-foreground underline decoration-foreground/45 underline-offset-4 hover:decoration-foreground"
                  data-testid="onboarding-preview-harness-help-trigger"
                  onClick={onHelp}
                  type="button"
                  variant="link"
                >
                  Need help choosing?
                </Button>
              ) : null}
            </p>
          )}
        </div>
        <div className="mt-6 flex min-h-0 flex-1 flex-col">
          <HarnessConnectionList
            installedIds={installedIds}
            method={method}
            onSelect={onSelect}
            options={options}
          />
        </div>
      </OnboardingSlideTransition>
    </HarnessPreviewStep>
  );
}

const CONNECTION_METHOD_CHOICES = [
  {
    icon: CreditCard,
    label: "Log in with a subscription",
    method: "subscription",
  },
  {
    icon: KeyRound,
    label: "Use an API key",
    method: "api",
  },
] as const satisfies ReadonlyArray<{
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  method: HarnessConnectionMethod;
}>;

export function HarnessConnectionMethodPreview({
  embedded = false,
  onBack,
  onSelect,
  onSetUpLater,
  total,
}: {
  embedded?: boolean;
  onBack: () => void;
  onSelect: (method: HarnessConnectionMethod) => void;
  onSetUpLater: () => void;
  total: number;
}) {
  return (
    <HarnessPreviewStep
      embedded={embedded}
      onBack={onBack}
      testId="onboarding-preview-harness-method"
      total={total}
    >
      <OnboardingSlideTransition
        className="flex min-h-0 flex-1 flex-col"
        containerClassName="min-h-0 flex-1"
        transitionKey="preview-harness-method"
      >
        <div className="shrink-0">
          <h1 className="text-title font-normal text-foreground">
            Connect your AI provider
          </h1>
          <p className="mt-2 text-base leading-6 text-foreground/80">
            Choose how your agents will access AI. You can change this later.
          </p>
        </div>

        <div className="-mx-2 mt-6 flex w-[calc(100%+1rem)] flex-1 flex-col gap-2">
          {CONNECTION_METHOD_CHOICES.map(
            ({ icon: MethodIcon, label, method }) => (
              <Button
                className="group h-auto min-h-14 w-full justify-start gap-3 rounded-xl px-2 py-2 text-left text-sm text-foreground shadow-none hover:bg-foreground/[0.04] hover:text-foreground focus-visible:ring-2 focus-visible:ring-foreground/20"
                data-testid={`onboarding-preview-harness-method-${method}`}
                key={method}
                onClick={() => onSelect(method)}
                type="button"
                variant="ghost"
              >
                <span className="flex size-8 shrink-0 items-center justify-start">
                  <MethodIcon aria-hidden className="!size-6" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block font-medium">{label}</span>
                </span>
                <span className="ml-auto flex size-10 shrink-0 items-center justify-end">
                  <ChevronRight
                    aria-hidden
                    className="size-4 text-muted-foreground transition-colors duration-150 ease-out group-hover:text-foreground motion-reduce:transition-none"
                  />
                </span>
              </Button>
            ),
          )}
          <Button
            className="group h-auto min-h-14 w-full justify-start gap-3 rounded-xl px-2 py-2 text-left text-sm font-medium text-foreground shadow-none hover:bg-foreground/[0.04] hover:text-foreground focus-visible:ring-2 focus-visible:ring-foreground/20"
            data-testid="onboarding-preview-harness-method-later"
            onClick={onSetUpLater}
            type="button"
            variant="ghost"
          >
            <span aria-hidden className="size-8 shrink-0" />
            <span className="min-w-0 flex-1">Set up later</span>
            <span className="ml-auto flex size-10 shrink-0 items-center justify-end">
              <ChevronRight
                aria-hidden
                className="size-4 text-muted-foreground transition-colors duration-150 ease-out group-hover:text-foreground motion-reduce:transition-none"
              />
            </span>
          </Button>
        </div>
      </OnboardingSlideTransition>
    </HarnessPreviewStep>
  );
}

function HarnessCompatibilityList({
  method,
}: {
  method: HarnessConnectionMethod;
}) {
  const options = HARNESS_CONNECTION_OPTIONS.filter(({ methods }) =>
    methods.includes(method),
  );

  return (
    <ul
      className="mt-4 grid grid-cols-2 gap-x-5 gap-y-3"
      data-testid={`onboarding-preview-harness-help-${method}-list`}
    >
      {options.map(({ runtime }) => (
        <li className="flex min-w-0 items-center gap-2" key={runtime.id}>
          <span className="flex size-7 shrink-0 items-center justify-center">
            <RuntimeIcon className="size-6" runtime={runtime} />
          </span>
          <span className="truncate text-sm font-medium text-foreground">
            {getRuntimeDisplayLabel(runtime)}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function HarnessConnectionHelpPreview({
  onBack,
  total,
}: {
  onBack: () => void;
  total: number;
}) {
  return (
    <OnboardingPreviewStep
      current={3}
      onBack={onBack}
      testId="onboarding-preview-harness-help"
      total={total}
    >
      <OnboardingSlideTransition
        className="flex min-h-0 w-full max-w-[500px] flex-1 flex-col items-stretch text-left"
        containerClassName="min-h-0 flex-1"
        transitionKey="preview-harness-help"
      >
        <div className="shrink-0">
          <h1 className="text-title font-normal text-foreground">
            Choose how to connect
          </h1>
          <p className="mt-2 text-base leading-6 text-foreground/80">
            Connect an AI client using a subscription or API key.
          </p>
        </div>

        <div className="mt-8 min-h-0 flex-1 space-y-7 overflow-y-auto pb-4 pr-1">
          <section aria-labelledby="harness-subscription-heading">
            <h2
              className="text-base font-medium text-foreground"
              id="harness-subscription-heading"
            >
              I use a subscription
            </h2>
            <p className="mt-2 text-sm leading-6 text-foreground/75">
              Choose this if you sign in to a paid AI product, such as ChatGPT
              or Claude.
            </p>
            <HarnessCompatibilityList method="subscription" />
          </section>

          <section
            aria-labelledby="harness-api-heading"
            className="border-t border-[#e2e2e2] pt-7"
          >
            <h2
              className="text-base font-medium text-foreground"
              id="harness-api-heading"
            >
              I use an API key
            </h2>
            <p className="mt-2 text-sm leading-6 text-foreground/75">
              Choose this if you have an API key from an AI provider or your
              team.
            </p>
            <HarnessCompatibilityList method="api" />
          </section>
        </div>
      </OnboardingSlideTransition>
    </OnboardingPreviewStep>
  );
}

const API_PROVIDER_OPTIONS = PERSONA_LLM_PROVIDER_OPTIONS.map((provider) => ({
  label: provider.label,
  value: provider.id,
}));

const SUBSCRIPTION_NAMES: Record<string, string> = {
  amp: "Amp subscription",
  claude: "Claude subscription",
  codex: "ChatGPT subscription",
  cursor: "Cursor subscription",
  devin: "Devin account",
};

function defaultApiProvider(runtimeId: string) {
  if (runtimeId === "codex") return "openai";
  return "anthropic";
}

export function runtimeNeedsOnboardingConnection(
  method: HarnessConnectionMethod,
  runtimeId: string,
) {
  return (
    method === "subscription" || runtimeSupportsLlmProviderSelection(runtimeId)
  );
}

function apiKeyLabel(provider: string) {
  return (
    getProviderApiKeyLabel(provider)?.replace("API Key", "API key") ?? "API key"
  );
}

const DEFAULT_MODEL_OPTION = { label: "Default model", value: "" } as const;

function modelOptions(runtimeId: string, provider: string) {
  const options = getPersonaModelOptions(runtimeId, provider).map((option) => ({
    label: option.label,
    value: option.id,
  }));
  return options.length > 0 ? options : [DEFAULT_MODEL_OPTION];
}

function ConnectedConfiguration({
  effort,
  model,
  onEffortChange,
  onModelChange,
  provider,
  runtimeId,
}: {
  effort: string;
  model: string;
  onEffortChange: (value: string) => void;
  onModelChange: (value: string) => void;
  provider: string;
  runtimeId: string;
}) {
  const models = modelOptions(runtimeId, provider);
  const effortConfig = getProviderEffortConfig(provider, model);
  const efforts = BUZZ_AGENT_THINKING_EFFORT_VALUES.filter((value) =>
    effortConfig.validValues.includes(value),
  ).map((value) => ({
    label: value === effortConfig.defaultValue ? `${value} (default)` : value,
    value,
  }));
  const availableEfforts =
    efforts.length > 0
      ? efforts
      : BUZZ_AGENT_THINKING_EFFORT_VALUES.map((value) => ({
          label: value,
          value,
        }));

  return (
    <div
      className="space-y-5"
      data-testid="onboarding-preview-harness-connected"
    >
      <div>
        <label
          className="mb-2 block text-sm font-medium text-foreground"
          htmlFor="onboarding-preview-harness-model"
        >
          Model
        </label>
        <AgentDropdownSelect
          className={ONBOARDING_PREVIEW_CARD_INPUT_CLASS}
          id="onboarding-preview-harness-model"
          onValueChange={onModelChange}
          options={models}
          testId="onboarding-preview-harness-model"
          value={model}
        />
      </div>
      <div>
        <label
          className="mb-2 block text-sm font-medium text-foreground"
          htmlFor="onboarding-preview-harness-effort"
        >
          Effort
        </label>
        <AgentDropdownSelect
          className={ONBOARDING_PREVIEW_CARD_INPUT_CLASS}
          id="onboarding-preview-harness-effort"
          onValueChange={onEffortChange}
          options={availableEfforts}
          testId="onboarding-preview-harness-effort"
          value={effort}
        />
      </div>
    </div>
  );
}

export function HarnessConnectionDetailPreview({
  embedded = false,
  installed,
  lockMethod = false,
  method,
  onBack,
  onCheckAgain,
  onContinue,
  onMethodChange,
  onUseDifferentHarness,
  option,
  total,
}: {
  embedded?: boolean;
  installed: boolean;
  lockMethod?: boolean;
  method: HarnessConnectionMethod;
  onBack: () => void;
  onCheckAgain: () => void;
  onContinue: () => void;
  onMethodChange: (method: HarnessConnectionMethod) => void;
  onUseDifferentHarness?: () => void;
  option: HarnessConnectionOption;
  total: number;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const label = getRuntimeDisplayLabel(option.runtime);
  const hasMethodChoice = option.methods.length > 1 && !lockMethod;
  const [provider, setProvider] = React.useState(() =>
    defaultApiProvider(option.runtime.id),
  );
  const [apiKey, setApiKey] = React.useState("workshop-preview-key");
  const [connected, setConnected] = React.useState(false);
  const [model, setModel] = React.useState("");
  const [effort, setEffort] = React.useState("medium");
  const canChooseProvider = runtimeSupportsLlmProviderSelection(
    option.runtime.id,
  );
  const subscriptionName = SUBSCRIPTION_NAMES[option.runtime.id] ?? label;
  const requiresApiKey = getProviderApiKeyLabel(provider) !== null;
  const canConnect =
    method === "subscription" || !requiresApiKey || apiKey.trim().length > 0;

  React.useEffect(() => {
    if (!option.methods.includes(method)) {
      onMethodChange(option.methods[0] ?? "api");
    }
  }, [method, onMethodChange, option.methods]);

  function handleMethodChange(nextMethod: HarnessConnectionMethod) {
    setApiKey("workshop-preview-key");
    setConnected(false);
    setProvider(defaultApiProvider(option.runtime.id));
    setModel("");
    setEffort("medium");
    onMethodChange(nextMethod);
  }

  function handleProviderChange(nextProvider: string) {
    setProvider(nextProvider);
    setApiKey("workshop-preview-key");
    setConnected(false);
    setModel("");
    setEffort("medium");
  }

  function handlePrimaryAction() {
    if (connected) {
      onContinue();
      return;
    }
    if (canConnect) setConnected(true);
  }

  if (!installed) {
    return (
      <HarnessPreviewStep
        embedded={embedded}
        onBack={onBack}
        testId="onboarding-preview-harness-setup-guide"
        total={total}
      >
        <OnboardingSlideTransition
          className={cn(
            "flex min-h-0 w-full max-w-[500px] flex-1 flex-col",
            cardLayout || embedded ? "items-stretch" : "items-center",
          )}
          containerClassName="min-h-0 flex-1"
          transitionKey={`preview-harness-setup-${option.runtime.id}`}
        >
          <div className="flex items-center gap-3">
            <span className="flex size-10 shrink-0 items-center justify-center">
              <RuntimeIcon className="size-9" runtime={option.runtime} />
            </span>
            <h1 className="text-title font-normal text-foreground">
              Set up {label}
            </h1>
          </div>
          <p className="mt-2 max-w-[440px] text-base leading-6 text-foreground/80">
            Follow the setup guide to install {label}. When you’re done, come
            back and check again.
          </p>

          <div className="mt-8 flex min-h-0 flex-1 flex-col">
            <div className="rounded-xl bg-[#e2e2e2]/30 px-4 py-4">
              <p className="text-sm font-medium text-foreground">
                CLI not detected
              </p>
              <Button
                className="mt-1 h-auto justify-start p-0 text-sm text-foreground underline decoration-foreground/45 underline-offset-4 hover:decoration-foreground"
                data-testid="onboarding-preview-harness-open-setup-guide"
                onClick={() =>
                  void openUrl(option.runtime.installInstructionsUrl)
                }
                type="button"
                variant="link"
              >
                Open setup guide
                <ExternalLink aria-hidden />
              </Button>
            </div>
          </div>

          <OnboardingFooter>
            <Button
              className={ONBOARDING_PRIMARY_CTA_CLASS}
              data-testid="onboarding-preview-harness-check-again"
              onClick={onCheckAgain}
              type="button"
            >
              Check again
            </Button>
          </OnboardingFooter>
        </OnboardingSlideTransition>
      </HarnessPreviewStep>
    );
  }

  return (
    <HarnessPreviewStep
      embedded={embedded}
      onBack={onBack}
      testId="onboarding-preview-harness-connection-detail"
      total={total}
    >
      <OnboardingSlideTransition
        className={cn(
          "flex min-h-0 w-full max-w-[500px] flex-1 flex-col",
          cardLayout || embedded ? "items-stretch" : "items-center",
        )}
        containerClassName="min-h-0 flex-1"
        transitionKey={`preview-harness-connection-${option.runtime.id}`}
      >
        <div className="flex items-center gap-3">
          <span className="flex size-10 shrink-0 items-center justify-center">
            <RuntimeIcon className="size-9" runtime={option.runtime} />
          </span>
          <h1 className="text-title font-normal text-foreground">
            {connected
              ? "Choose your model settings"
              : method === "api" && option.runtime.id === "buzz-agent"
                ? "Connect with an API key"
                : `Connect ${label}`}
          </h1>
        </div>
        <p className="mt-2 max-w-[440px] text-base leading-6 text-foreground/80">
          {connected
            ? "Select the model and effort level your agents will use by default."
            : hasMethodChoice
              ? "Choose how you want to connect. You can change this anytime."
              : method === "subscription"
                ? `Sign in to connect ${label}. You can change this anytime.`
                : option.runtime.id === "buzz-agent"
                  ? "Choose your provider and enter an API key to connect to the Buzz harness."
                  : "Add your connection details. You can change this anytime."}
        </p>

        <div className="mt-8 flex min-h-0 flex-1 flex-col gap-6">
          {hasMethodChoice && !connected ? (
            <SegmentedControl
              appearance="onboarding-inline"
              className="w-fit self-start"
              legend={`Connection method for ${label}`}
              onValueChange={handleMethodChange}
              optionTestIdPrefix="onboarding-preview-harness-method"
              options={CONNECTION_METHOD_OPTIONS.filter((item) =>
                option.methods.includes(item.value),
              )}
              size="compact"
              testId="onboarding-preview-harness-methods"
              value={method}
            />
          ) : null}

          {connected ? (
            <ConnectedConfiguration
              effort={effort}
              model={model}
              onEffortChange={setEffort}
              onModelChange={setModel}
              provider={provider}
              runtimeId={option.runtime.id}
            />
          ) : method === "subscription" ? (
            <div
              className="flex items-center gap-3 rounded-xl bg-[#e2e2e2]/30 px-4 py-4"
              data-testid="onboarding-preview-harness-subscription"
            >
              <RuntimeIcon className="size-9" runtime={option.runtime} />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-foreground">
                  {subscriptionName}
                </p>
                <p className="mt-0.5 text-xs leading-5 text-foreground/70">
                  Buzz will open a sign-in window for {label}.
                </p>
              </div>
              <Button
                className="ml-auto h-7 shrink-0 rounded-full bg-foreground px-3 text-xs text-background shadow-none hover:bg-foreground/85"
                data-testid="onboarding-preview-harness-subscription-sign-in"
                onClick={() => setConnected(true)}
                size="xs"
                type="button"
              >
                Sign in
              </Button>
            </div>
          ) : (
            <form
              className="space-y-5"
              data-testid="onboarding-preview-harness-api-form"
              onSubmit={(event) => {
                event.preventDefault();
                handlePrimaryAction();
              }}
            >
              {canChooseProvider ? (
                <div>
                  <label
                    className="mb-2 block text-sm font-medium text-foreground"
                    htmlFor="onboarding-preview-harness-provider"
                  >
                    Provider
                  </label>
                  <AgentDropdownSelect
                    className={ONBOARDING_PREVIEW_CARD_INPUT_CLASS}
                    id="onboarding-preview-harness-provider"
                    onValueChange={handleProviderChange}
                    options={API_PROVIDER_OPTIONS}
                    testId="onboarding-preview-harness-provider"
                    value={provider}
                  />
                </div>
              ) : null}
              {requiresApiKey ? (
                <div>
                  <label
                    className="mb-2 block text-sm font-medium text-foreground"
                    htmlFor="onboarding-preview-harness-api-key"
                  >
                    {apiKeyLabel(provider)}
                  </label>
                  <OnboardingPreviewInput
                    autoComplete="off"
                    className={ONBOARDING_PREVIEW_CARD_INPUT_CLASS}
                    id="onboarding-preview-harness-api-key"
                    onChange={(event) => {
                      setApiKey(event.target.value);
                      setConnected(false);
                    }}
                    placeholder="Paste API key"
                    smooth
                    type="password"
                    value={apiKey}
                  />
                </div>
              ) : null}
              {option.runtime.id === "buzz-agent" && onUseDifferentHarness ? (
                <div className="flex w-full items-baseline gap-1.5 pt-1 text-sm text-foreground/70">
                  <span>or</span>
                  <Button
                    className="h-auto p-0 text-sm text-foreground"
                    data-testid="onboarding-preview-harness-use-different"
                    onClick={onUseDifferentHarness}
                    type="button"
                    variant="link"
                  >
                    Use a different harness
                  </Button>
                </div>
              ) : null}
            </form>
          )}
        </div>

        {method === "api" || connected ? (
          <OnboardingFooter>
            <Button
              className={ONBOARDING_PRIMARY_CTA_CLASS}
              data-testid="onboarding-preview-harness-continue"
              disabled={!canConnect}
              onClick={handlePrimaryAction}
              type="button"
            >
              {connected ? "Continue" : "Connect"}
            </Button>
          </OnboardingFooter>
        ) : null}
      </OnboardingSlideTransition>
    </HarnessPreviewStep>
  );
}
