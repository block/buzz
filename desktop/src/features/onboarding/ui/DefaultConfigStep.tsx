import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpAuthMethodsQuery,
  useAcpRuntimesQuery,
  useConnectAcpRuntimeMutation,
  useInstallAcpRuntimeMutation,
  useRuntimeFileConfigQuery,
} from "@/features/agents/hooks";
import {
  AgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/AgentConfigFields";
import { resetConfigForHarnessChange } from "@/features/agents/ui/agentConfigOptions";
import { getRuntimeBakedDefaults } from "@/features/agents/ui/bakedEnvHelpers";
import { getGlobalAgentCredentialState } from "@/features/agents/ui/globalAgentCredentialState";
import { getBakedBuildEnv, type BakedEnvEntry } from "@/shared/api/tauri";
import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import type {
  AcpAuthMethod,
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import {
  HARNESS_SETUP_CARD_WIDTH_CLASS,
  HarnessChoiceCard,
} from "./HarnessChoiceCard";
import { RuntimeErrorTooltip } from "./RuntimeErrorTooltip";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import {
  getVisibleOnboardingRuntimes,
  runtimeIsReadyForOnboarding,
} from "./onboardingRuntimeSelection";
import { getRuntimeDisplayLabel } from "./RuntimeIcon";
import type { DefaultConfigStepActions } from "./types";

type DefaultConfigStepProps = {
  actions: DefaultConfigStepActions;
  direction: OnboardingTransitionDirection;
  selectedRuntimeId: string | null;
};

type InstallResultState = {
  error: string | null;
  success: boolean;
};

type SetupPlanKind =
  | "loading"
  | "catalog-error"
  | "missing-runtime"
  | "needs-install"
  | "needs-sign-in"
  | "auth-unknown"
  | "auth-invalid"
  | "needs-provider-defaults"
  | "ready";

function runtimeNeedsOnboardingProviderSetup(runtime: AcpRuntimeCatalogEntry) {
  return Boolean(runtime.providerEnvVar);
}

function bakedEnvKeys(bakedEnv: readonly BakedEnvEntry[]) {
  return bakedEnv.map((entry) => entry.key);
}

function providerDefaultsAreSatisfied({
  bakedEnv,
  config,
  runtimeFileConfig,
  runtime,
}: {
  bakedEnv: readonly BakedEnvEntry[];
  config: GlobalAgentConfig;
  runtimeFileConfig:
    | {
        provider: string | null;
        model: string | null;
        satisfiedEnvKeys: string[];
      }
    | null
    | undefined;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const runtimeBakedDefaults = getRuntimeBakedDefaults(
    bakedEnv,
    runtime,
    config.env_vars,
  );
  const effectiveProvider =
    config.provider?.trim() ||
    runtimeBakedDefaults.provider ||
    runtimeFileConfig?.provider?.trim() ||
    "";
  const effectiveModel =
    config.model?.trim() ||
    runtimeFileConfig?.model?.trim() ||
    runtimeBakedDefaults.model ||
    "";
  const { credentialsValid } = getGlobalAgentCredentialState({
    bakedEnvKeys: bakedEnvKeys(bakedEnv),
    envVars: config.env_vars,
    provider: effectiveProvider,
    runtimeFileConfig,
    runtimeId: runtime.id,
  });

  return (
    effectiveProvider.length > 0 &&
    effectiveModel.length > 0 &&
    credentialsValid
  );
}

function deriveSetupPlan({
  bakedEnv,
  config,
  isConfigLoading,
  runtime,
  runtimeFileConfig,
}: {
  bakedEnv: readonly BakedEnvEntry[];
  config: GlobalAgentConfig;
  isConfigLoading: boolean;
  runtime: AcpRuntimeCatalogEntry | undefined;
  runtimeFileConfig:
    | {
        provider: string | null;
        model: string | null;
        satisfiedEnvKeys: string[];
      }
    | null
    | undefined;
}): SetupPlanKind {
  if (isConfigLoading) return "loading";
  if (!runtime) return "missing-runtime";
  if (runtime.availability !== "available") return "needs-install";
  if (runtime.authStatus.status === "logged_out") return "needs-sign-in";
  if (runtime.authStatus.status === "unknown") return "auth-unknown";
  if (runtime.authStatus.status === "config_invalid") return "auth-invalid";

  if (runtimeNeedsOnboardingProviderSetup(runtime)) {
    if (
      providerDefaultsAreSatisfied({
        bakedEnv,
        config,
        runtimeFileConfig,
        runtime,
      })
    ) {
      return "ready";
    }
    return "needs-provider-defaults";
  }

  return runtimeIsReadyForOnboarding(runtime) ? "ready" : "auth-unknown";
}

function isSupportedOnboardingAuthMethod(
  runtime: AcpRuntimeCatalogEntry,
  method: AcpAuthMethod,
) {
  if (runtime.id !== "codex") return true;
  return !/api[-_ ]?key/i.test(`${method.id} ${method.name}`);
}

function isPreferredClaudeAuthMethod(method: AcpAuthMethod) {
  const haystack = [
    method.id,
    method.name,
    method.description ?? "",
    method.command.join(" "),
    method.args.join(" "),
  ]
    .join(" ")
    .toLowerCase();
  return (
    haystack.includes("claudeai") ||
    haystack.includes("claude ai") ||
    haystack.includes("claude.ai") ||
    haystack.includes("subscription")
  );
}

function getOnboardingAuthMethods(
  runtime: AcpRuntimeCatalogEntry,
  methods: AcpAuthMethod[],
) {
  const supported = methods.filter((method) =>
    isSupportedOnboardingAuthMethod(runtime, method),
  );
  if (runtime.id === "claude") {
    const preferred =
      supported.find(isPreferredClaudeAuthMethod) ?? supported[0];
    return preferred ? [preferred] : [];
  }
  if (runtime.id === "codex") return supported.slice(0, 1);
  return supported;
}

function SelectedRuntimeHeader({
  runtime,
}: {
  runtime: AcpRuntimeCatalogEntry | undefined;
}) {
  return (
    <div className="w-full max-w-[540px] text-center">
      <h1 className="text-title font-normal text-foreground">
        Set up {runtime ? getRuntimeDisplayLabel(runtime) : "your harness"}
      </h1>
      <p className="mx-auto mt-3 max-w-[440px] text-sm leading-5 text-foreground/80">
        Buzz will use this harness by default. You can connect and add more
        harnesses later in Settings.
      </p>
    </div>
  );
}

/**
 * Setup-page layout: the selected harness card sits on the left and the
 * state-specific description + action live in a left-aligned column on the
 * right. The parent page owns the single vanilla fade transition.
 */
function SelectedHarnessLayout({
  children,
  runtime,
}: {
  children: React.ReactNode;
  runtime: AcpRuntimeCatalogEntry;
}) {
  return (
    <div className="flex w-full flex-col items-center gap-8 sm:flex-row sm:items-center sm:justify-center sm:gap-14">
      <div
        className={cn("shrink-0", HARNESS_SETUP_CARD_WIDTH_CLASS)}
        data-testid="onboarding-selected-runtime-card"
      >
        <HarnessChoiceCard runtime={runtime} />
      </div>
      <div className="flex w-full max-w-2xs flex-col items-start gap-4 text-left [@media(max-height:560px)]:gap-2">
        {children}
      </div>
    </div>
  );
}

function LoadingSetup({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  return (
    <SelectedHarnessLayout runtime={runtime}>
      <div
        aria-live="polite"
        className="flex items-center gap-2 text-sm text-muted-foreground"
        role="status"
      >
        <Spinner className="h-4 w-4 border-2" />
        Checking setup…
      </div>
    </SelectedHarnessLayout>
  );
}

function InstallSetup({
  installResult,
  isInstalling,
  onInstall,
  runtime,
}: {
  installResult: InstallResultState;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const installLabel = installResult.error ? "Retry install" : "Install";

  return (
    <SelectedHarnessLayout runtime={runtime}>
      <p className="text-sm leading-6 text-foreground/80">
        {getRuntimeDisplayLabel(runtime)} hasn’t been detected on your computer.
        You can install it now.
      </p>
      {runtime.canAutoInstall ? (
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid={`onboarding-runtime-install-${runtime.id}`}
          disabled={isInstalling}
          onClick={onInstall}
          type="button"
        >
          {isInstalling ? (
            <span className="inline-flex items-center gap-2">
              <Spinner className="h-4 w-4 border-2 text-foreground" />
              Installing…
            </span>
          ) : (
            installLabel
          )}
        </Button>
      ) : (
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          onClick={() => void openUrl(runtime.installInstructionsUrl)}
          type="button"
        >
          View install instructions
        </Button>
      )}
      {installResult.error ? (
        <RuntimeErrorTooltip
          className="flex min-w-0 items-center gap-1.5 overflow-hidden whitespace-nowrap text-xs leading-4 text-destructive"
          detail={installResult.error}
          label="Installation failed"
          showIcon
          testId={`onboarding-runtime-error-${runtime.id}`}
        />
      ) : null}
    </SelectedHarnessLayout>
  );
}

function SignInSetup({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  const methodsQuery = useAcpAuthMethodsQuery(runtime.id, {
    enabled:
      runtime.availability === "available" &&
      runtime.authStatus.status === "logged_out",
  });
  const connectMutation = useConnectAcpRuntimeMutation();
  const runtimesQuery = useAcpRuntimesQuery();
  const [isWaitingForSignIn, setIsWaitingForSignIn] = React.useState(false);
  const [didSignInCheckTimeOut, setDidSignInCheckTimeOut] =
    React.useState(false);
  const isReady = runtimeIsReadyForOnboarding(runtime);

  React.useEffect(() => {
    if (!isWaitingForSignIn || !isReady) return;
    setIsWaitingForSignIn(false);
    setDidSignInCheckTimeOut(false);
  }, [isReady, isWaitingForSignIn]);

  React.useEffect(() => {
    if (!isWaitingForSignIn) return;

    const interval = window.setInterval(() => {
      void runtimesQuery.refetch();
    }, 2_000);
    const timeout = window.setTimeout(() => {
      setIsWaitingForSignIn(false);
      setDidSignInCheckTimeOut(true);
    }, 120_000);

    return () => {
      window.clearInterval(interval);
      window.clearTimeout(timeout);
    };
  }, [isWaitingForSignIn, runtimesQuery.refetch]);

  const authMethods = getOnboardingAuthMethods(
    runtime,
    methodsQuery.data?.methods ?? [],
  );
  const authMethod = authMethods[0] ?? null;
  const authMethodsUnavailable =
    !methodsQuery.isLoading && !methodsQuery.error && authMethod === null;

  return (
    <SelectedHarnessLayout runtime={runtime}>
      <p className="text-sm leading-6 text-foreground/80">
        Sign in to {getRuntimeDisplayLabel(runtime)} so Buzz can start agents
        with this harness.
      </p>
      <Button
        aria-label={`Sign in to ${runtime.label}`}
        className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
        data-testid={`onboarding-runtime-instructions-${runtime.id}`}
        disabled={connectMutation.isPending || authMethodsUnavailable}
        onClick={() => {
          if (didSignInCheckTimeOut) {
            setDidSignInCheckTimeOut(false);
            setIsWaitingForSignIn(true);
            void runtimesQuery.refetch();
            return;
          }
          if (!authMethod) {
            void methodsQuery.refetch();
            return;
          }
          connectMutation.mutate(
            { methodId: authMethod.id, runtimeId: runtime.id },
            { onSuccess: () => setIsWaitingForSignIn(true) },
          );
        }}
        type="button"
      >
        {isWaitingForSignIn
          ? "Checking…"
          : didSignInCheckTimeOut
            ? "Check again"
            : "Sign in"}
      </Button>
      {authMethodsUnavailable ? (
        <p className="text-xs leading-4 text-destructive" role="status">
          No supported sign-in method is available.
        </p>
      ) : null}
      {methodsQuery.error instanceof Error ? (
        <RuntimeErrorTooltip
          className="text-xs leading-4 text-destructive"
          detail="Couldn’t load sign-in options."
          label="Sign-in unavailable"
        />
      ) : null}
      {connectMutation.error instanceof Error ? (
        <RuntimeErrorTooltip
          className="text-xs leading-4 text-destructive"
          detail="Couldn’t start sign-in. Try again."
          label="Sign-in failed"
        />
      ) : null}
    </SelectedHarnessLayout>
  );
}

function AuthCheckSetup({
  label,
  message,
  runtime,
}: {
  label: string;
  message: string;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const runtimesQuery = useAcpRuntimesQuery();

  return (
    <SelectedHarnessLayout runtime={runtime}>
      <p className="text-sm leading-6 text-foreground/80">{message}</p>
      <Button
        className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
        disabled={runtimesQuery.isFetching}
        onClick={() => void runtimesQuery.refetch()}
        type="button"
      >
        {runtimesQuery.isFetching ? "Checking…" : label}
      </Button>
    </SelectedHarnessLayout>
  );
}

function ProviderDefaultsSetup({
  bakedEnv,
  config,
  onConfigChange,
  onValidityChange,
  runtime,
  runtimeFileConfig,
}: {
  bakedEnv: BakedEnvEntry[];
  config: GlobalAgentConfig;
  onConfigChange: (next: GlobalAgentConfig) => void;
  onValidityChange: (valid: boolean) => void;
  runtime: AcpRuntimeCatalogEntry;
  runtimeFileConfig:
    | {
        provider: string | null;
        model: string | null;
        satisfiedEnvKeys: string[];
      }
    | null
    | undefined;
}) {
  const [isCustomProvider, setIsCustomProvider] = React.useState(false);
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);

  return (
    <SelectedHarnessLayout runtime={runtime}>
      <p className="text-sm leading-6 text-foreground/80">
        Choose the default provider and model Buzz should use with this harness.
      </p>
      <div className="w-full">
        <AgentConfigFields
          bakedEnv={bakedEnv}
          selectedRuntime={runtime}
          config={config}
          isCustomModelEditing={isCustomModelEditing}
          isCustomProvider={isCustomProvider}
          onConfigChange={onConfigChange}
          onCustomModelEditingChange={setIsCustomModelEditing}
          onIsCustomProviderChange={setIsCustomProvider}
          onValidityChange={onValidityChange}
          placeholderClassName="text-foreground/70"
          runtimeFileConfig={runtimeFileConfig}
          selectClassName="h-12 rounded-2xl border-foreground/15 bg-white px-4 py-2 text-sm shadow-none hover:bg-white/95"
          disclosure="onboarding-essential"
          unstyled
          useCustomSelect
        />
      </div>
    </SelectedHarnessLayout>
  );
}

function ReadySetup({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  return (
    <SelectedHarnessLayout runtime={runtime}>
      <p className="text-sm leading-6 text-foreground/80">
        {getRuntimeDisplayLabel(runtime)} is ready. Buzz will use it as your
        default harness.
      </p>
    </SelectedHarnessLayout>
  );
}

export function DefaultConfigStep({
  actions,
  direction,
  selectedRuntimeId,
}: DefaultConfigStepProps) {
  const runtimesQuery = useAcpRuntimesQuery();
  const visibleRuntimes = React.useMemo(
    () => getVisibleOnboardingRuntimes(runtimesQuery.data ?? []),
    [runtimesQuery.data],
  );
  const [config, setConfig] =
    React.useState<GlobalAgentConfig>(EMPTY_GLOBAL_CONFIG);
  // Reopening machine config from the first-community flow starts directly on
  // this page, without a chooser draft. Fall back to the persisted default so
  // the same selected-harness setup renders instead of a missing-runtime state.
  const effectiveSelectedRuntimeId =
    selectedRuntimeId ?? config.preferred_runtime;
  const selectedRuntime = visibleRuntimes.find(
    (runtime) => runtime.id === effectiveSelectedRuntimeId,
  );
  const { data: runtimeFileConfig, isLoading: runtimeFileConfigLoading } =
    useRuntimeFileConfigQuery(effectiveSelectedRuntimeId ?? "", {
      enabled: Boolean(effectiveSelectedRuntimeId),
    });
  const [bakedEnv, setBakedEnv] = React.useState<BakedEnvEntry[]>([]);
  const [isConfigLoading, setIsConfigLoading] = React.useState(true);
  const [configValid, setConfigValid] = React.useState(false);
  const [installResult, setInstallResult] = React.useState<InstallResultState>({
    error: null,
    success: false,
  });
  const installMutation = useInstallAcpRuntimeMutation();
  const [completionError, setCompletionError] = React.useState<string | null>(
    null,
  );
  const [isCompleting, setIsCompleting] = React.useState(false);

  React.useEffect(() => {
    let unmounted = false;

    async function loadDefaults() {
      const [configResult, bakedEnvResult] = await Promise.allSettled([
        getGlobalAgentConfig(),
        getBakedBuildEnv(),
      ]);

      if (unmounted) return;

      if (configResult.status === "fulfilled") {
        const harnessChanged =
          selectedRuntimeId !== null &&
          selectedRuntimeId !== configResult.value.preferred_runtime;
        setConfig(
          harnessChanged
            ? resetConfigForHarnessChange(configResult.value, selectedRuntimeId)
            : configResult.value,
        );
      }
      if (bakedEnvResult.status === "fulfilled") {
        setBakedEnv(bakedEnvResult.value);
      }
      setIsConfigLoading(false);
    }

    void loadDefaults();

    return () => {
      unmounted = true;
    };
  }, [selectedRuntimeId]);

  const plan: SetupPlanKind = runtimesQuery.isError
    ? "catalog-error"
    : deriveSetupPlan({
        bakedEnv,
        config,
        isConfigLoading:
          isConfigLoading ||
          runtimesQuery.isLoading ||
          runtimeFileConfigLoading,
        runtime: selectedRuntime,
        runtimeFileConfig,
      });
  // Once this page has disclosed provider setup, keep those fields mounted as
  // they become valid. Validity gates Finish; it must not replace the form the
  // user is actively completing.
  const [providerSetupActivated, setProviderSetupActivated] =
    React.useState(false);
  React.useEffect(() => {
    if (plan === "needs-provider-defaults") setProviderSetupActivated(true);
  }, [plan]);
  const effectivePlan =
    providerSetupActivated &&
    plan === "ready" &&
    selectedRuntime &&
    runtimeNeedsOnboardingProviderSetup(selectedRuntime)
      ? "needs-provider-defaults"
      : plan;
  const canFinish =
    effectivePlan === "ready" ||
    (effectivePlan === "needs-provider-defaults" && configValid);

  const handleInstall = React.useCallback(() => {
    if (!selectedRuntime) return;
    setInstallResult({ error: null, success: false });
    installMutation.mutate(selectedRuntime.id, {
      onSuccess: (result) => {
        if (result.success) {
          setInstallResult({ error: null, success: true });
          void runtimesQuery.refetch();
          return;
        }
        setInstallResult({
          error: getInstallErrorMessage(result.steps),
          success: false,
        });
      },
      onError: (error) => {
        setInstallResult({
          error: error instanceof Error ? error.message : "Install failed.",
          success: false,
        });
      },
    });
  }, [installMutation, runtimesQuery, selectedRuntime]);

  const handleComplete = React.useCallback(async () => {
    if (!effectiveSelectedRuntimeId) return;
    setIsCompleting(true);
    setCompletionError(null);
    try {
      await setGlobalAgentConfig({
        ...config,
        preferred_runtime: effectiveSelectedRuntimeId,
      });
      actions.complete();
    } catch {
      setCompletionError("Couldn't save your default harness. Try again.");
      setIsCompleting(false);
    }
  }, [actions, config, effectiveSelectedRuntimeId]);

  return (
    <OnboardingSlideTransition
      className="flex min-h-full w-full flex-col items-center"
      data-testid="onboarding-page-config"
      direction={direction}
      effect="fade"
      transitionKey={`default-config-${direction}`}
    >
      <SelectedRuntimeHeader runtime={selectedRuntime} />

      <div className="flex w-full flex-1 items-center justify-center py-6 [@media(max-height:560px)]:items-start [@media(max-height:560px)]:py-2">
        <div className="w-full max-w-[760px]">
          {effectivePlan === "loading" && selectedRuntime ? (
            <LoadingSetup runtime={selectedRuntime} />
          ) : effectivePlan === "loading" ? (
            <div className="flex items-center justify-center gap-2 py-4 text-sm text-muted-foreground">
              <Spinner className="h-4 w-4 border-2" />
              Loading…
            </div>
          ) : effectivePlan === "catalog-error" ? (
            <div className="flex flex-col items-center gap-3 py-4 text-center text-sm">
              <p className="text-destructive">
                Couldn't load harness settings. Try again.
              </p>
              <Button
                className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
                disabled={runtimesQuery.isFetching}
                onClick={() => void runtimesQuery.refetch()}
                type="button"
              >
                {runtimesQuery.isFetching ? "Checking…" : "Try again"}
              </Button>
            </div>
          ) : effectivePlan === "missing-runtime" ? (
            <p className="py-4 text-center text-sm text-destructive">
              Couldn't load this harness. Go back and try again.
            </p>
          ) : selectedRuntime ? (
            effectivePlan === "needs-install" ? (
              <InstallSetup
                installResult={installResult}
                isInstalling={installMutation.isPending}
                onInstall={handleInstall}
                runtime={selectedRuntime}
              />
            ) : effectivePlan === "needs-sign-in" ? (
              <SignInSetup runtime={selectedRuntime} />
            ) : effectivePlan === "auth-unknown" ? (
              <AuthCheckSetup
                label="Check again"
                message="Buzz couldn't verify this harness yet. Check again after you've finished setup."
                runtime={selectedRuntime}
              />
            ) : effectivePlan === "auth-invalid" ? (
              <AuthCheckSetup
                label="Check again"
                message="This harness's configuration looks invalid. Fix it outside Buzz, then check again."
                runtime={selectedRuntime}
              />
            ) : effectivePlan === "needs-provider-defaults" ? (
              <ProviderDefaultsSetup
                bakedEnv={bakedEnv}
                config={config}
                onConfigChange={setConfig}
                onValidityChange={setConfigValid}
                runtime={selectedRuntime}
                runtimeFileConfig={runtimeFileConfig}
              />
            ) : (
              <ReadySetup runtime={selectedRuntime} />
            )
          ) : null}
          {completionError ? (
            <p
              className="mt-3 text-center text-xs text-destructive"
              role="alert"
            >
              {completionError}
            </p>
          ) : null}
        </div>
      </div>

      <OnboardingFooter>
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid="onboarding-finish"
          disabled={!canFinish || isCompleting}
          onClick={() => void handleComplete()}
          type="button"
        >
          Finish
        </Button>

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 text-sm hover:bg-foreground/15"
          data-testid="onboarding-back"
          onClick={actions.back}
          type="button"
          variant="ghost"
        >
          Back
        </Button>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
