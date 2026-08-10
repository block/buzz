import * as React from "react";
import {
  CheckCircle2,
  CircleAlert,
  CircleDashed,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";

import {
  useInstallNxtlinqAuthorizationGatewayMutation,
  useNxtlinqAuthorizationConfigQuery,
  useNxtlinqAuthorizationGatewayQuery,
  useNxtlinqAuthorizationSetupQuery,
} from "@/features/agents/nxtlinqHooks";
import {
  buildNxtlinqWrapperArgs,
  deriveNxtlinqReceiptDirectory,
  isNxtlinqGatewayCommand,
  parseNxtlinqLaunchPreset,
  NXTLINQ_GATEWAY_COMMAND,
  shouldBlockNxtlinqLaunchSave,
} from "@/features/agents/lib/nxtlinqLaunchPreset";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import type { EnvVarsValue } from "./EnvVarsEditor";
import type { AgentLaunchFields } from "./useAgentLaunchFields";

export function NxtlinqAuthorizationPreset({
  agentPubkey,
  disabled,
  envVars,
  inheritedEnvVars,
  launchFields,
  onEnvVarsChange,
  onSaveBlockedChange,
  requiredEnvKeys,
}: {
  agentPubkey: string;
  disabled: boolean;
  envVars: EnvVarsValue;
  inheritedEnvVars: Record<string, string>;
  launchFields: AgentLaunchFields;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onSaveBlockedChange: (blocked: boolean) => void;
  requiredEnvKeys: readonly string[];
}) {
  const gatewayQuery = useNxtlinqAuthorizationGatewayQuery();
  const configQuery = useNxtlinqAuthorizationConfigQuery();
  const installMutation = useInstallNxtlinqAuthorizationGatewayMutation();
  const enabled = isNxtlinqGatewayCommand(launchFields.commandWrapperCommand);
  const workspace = launchFields.workingDirectory.trim();
  const trustStore = configQuery.data?.trustStore?.trim() ?? "";
  const receiptDirectory = React.useMemo(
    () =>
      deriveNxtlinqReceiptDirectory(
        configQuery.data?.receiptRoot ?? "",
        agentPubkey,
      ),
    [agentPubkey, configQuery.data?.receiptRoot],
  );
  const setupConfiguration = React.useMemo(
    () =>
      JSON.stringify([workspace, trustStore.trim(), receiptDirectory.trim()]),
    [receiptDirectory, trustStore, workspace],
  );
  const [checkedConfiguration, setCheckedConfiguration] = React.useState<
    string | null
  >(null);
  const setupQuery = useNxtlinqAuthorizationSetupQuery({
    projectRoot: workspace,
    trustStore: trustStore.trim(),
    receiptDirectory: receiptDirectory.trim(),
    enabled: false,
  });

  const pathsComplete =
    workspace.length > 0 &&
    trustStore.trim().length > 0 &&
    receiptDirectory.trim().length > 0;
  const setupIsCurrent = checkedConfiguration === setupConfiguration;
  const currentSetup = setupIsCurrent ? setupQuery.data : undefined;
  const canEnable = pathsComplete && currentSetup?.ready === true;
  const draftPreset = React.useMemo(
    () => ({
      project: workspace,
      trustStore: trustStore.trim(),
      receiptDirectory: receiptDirectory.trim(),
    }),
    [receiptDirectory, trustStore, workspace],
  );
  const appliedPreset = React.useMemo(
    () =>
      parseNxtlinqLaunchPreset(
        launchFields.commandWrapperArgs
          .split(",")
          .map((value) => value.trim())
          .filter(Boolean),
      ),
    [launchFields.commandWrapperArgs],
  );
  const saveBlocked = shouldBlockNxtlinqLaunchSave({
    enabled,
    appliedPreset,
    draftPreset,
    draftVerified: canEnable,
  });

  React.useEffect(() => {
    if (
      checkedConfiguration !== null &&
      checkedConfiguration !== setupConfiguration
    ) {
      setCheckedConfiguration(null);
    }
  }, [checkedConfiguration, setupConfiguration]);

  React.useEffect(() => {
    onSaveBlockedChange(saveBlocked);
  }, [onSaveBlockedChange, saveBlocked]);

  async function recheckSetup(
    gatewayAvailable = gatewayQuery.data?.available === true,
  ) {
    if (!pathsComplete || !gatewayAvailable) return;
    const configuration = setupConfiguration;
    const result = await setupQuery.refetch();
    if (!result.isError) setCheckedConfiguration(configuration);
    if (result.data?.ready && enabled) applyPreset();
    return result.data;
  }

  function applyPreset(command?: string) {
    const passEnvironment = [
      ...requiredEnvKeys,
      ...Object.keys({ ...inheritedEnvVars, ...envVars }),
    ];
    const args = buildNxtlinqWrapperArgs({
      project: workspace,
      trustStore: trustStore.trim(),
      receiptDirectory: receiptDirectory.trim(),
      passEnvironment,
    });
    launchFields.setCommandWrapperCommand(
      command || gatewayQuery.data?.resolvedPath || NXTLINQ_GATEWAY_COMMAND,
    );
    launchFields.setCommandWrapperArgs(args.join(","));
    const nextEnv: EnvVarsValue = {
      ...envVars,
      BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE: "1",
      BUZZ_AGENT_REQUIRE_REPLY: envVars.BUZZ_AGENT_REQUIRE_REPLY || "1",
    };
    delete nextEnv.BUZZ_ACP_TRUST_NXTLINQ_GATEWAY;
    onEnvVarsChange(nextEnv);
  }

  function disablePreset() {
    launchFields.setCommandWrapperCommand("");
    launchFields.setCommandWrapperArgs("");
    const next = { ...envVars };
    delete next.BUZZ_ACP_TRUST_NXTLINQ_GATEWAY;
    if (inheritedEnvVars.BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE) {
      next.BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE = "0";
    } else {
      delete next.BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE;
    }
    onEnvVarsChange(next);
  }

  async function installAndEnable() {
    try {
      const result = await installMutation.mutateAsync(false);
      if (!result.success) return;
      const discovery = await gatewayQuery.refetch();
      const setup = await recheckSetup(discovery.data?.available === true);
      if (setup?.ready) {
        applyPreset(discovery.data?.resolvedPath ?? undefined);
      }
    } catch {
      // The mutation/query state renders the actionable failure below.
    }
  }

  const failure =
    installMutation.data?.success === false
      ? installMutation.data.steps.at(-1)?.hint ||
        installMutation.data.steps.at(-1)?.stderr ||
        "Nxtlinq Gateway installation failed."
      : installMutation.error instanceof Error
        ? installMutation.error.message
        : null;

  return (
    <div className="space-y-4 rounded-xl border border-border/80 bg-muted/20 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1">
          <div className="flex items-center gap-2 text-sm font-semibold text-foreground">
            <ShieldCheck className="size-4" />
            Nxtlinq authorization
          </div>
          <p className="text-xs text-muted-foreground">
            Enforce the signed policy for this workspace without entering raw
            wrapper arguments.
          </p>
        </div>
        {enabled ? (
          <Button
            disabled={disabled}
            onClick={disablePreset}
            size="sm"
            type="button"
            variant="outline"
          >
            Disable
          </Button>
        ) : gatewayQuery.data?.available ? (
          <Button
            disabled={disabled || !canEnable}
            onClick={() => applyPreset()}
            size="sm"
            type="button"
          >
            Enable
          </Button>
        ) : (
          <Button
            disabled={disabled || !pathsComplete || installMutation.isPending}
            onClick={() => void installAndEnable()}
            size="sm"
            type="button"
          >
            {installMutation.isPending ? (
              <LoaderCircle className="mr-2 size-4 animate-spin" />
            ) : null}
            Install & enable
          </Button>
        )}
      </div>

      {gatewayQuery.data?.available ? (
        <div className="space-y-2 rounded-lg border border-border/60 bg-background/60 p-3">
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs font-semibold text-foreground">
              Setup readiness
            </span>
            <Button
              disabled={disabled || !pathsComplete || setupQuery.isFetching}
              onClick={() => void recheckSetup()}
              size="sm"
              type="button"
              variant="ghost"
            >
              <RefreshCw
                className={cn(
                  "mr-1.5 size-3.5",
                  setupQuery.isFetching && "animate-spin",
                )}
              />
              Recheck
            </Button>
          </div>
          {setupQuery.isFetching ? (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <LoaderCircle className="size-3.5 animate-spin" />
              Verifying signed policy…
            </div>
          ) : !setupIsCurrent ? (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <CircleDashed className="size-3.5" />
              Paths changed. Select Recheck to verify this setup.
            </div>
          ) : (
            <ul className="space-y-2">
              {(currentSetup?.checks ?? []).map((check) => {
                const healthy = [
                  "ready",
                  "found",
                  "valid",
                  "willCreate",
                ].includes(check.status);
                const blocked = check.status === "blocked";
                const Icon = healthy
                  ? CheckCircle2
                  : blocked
                    ? CircleDashed
                    : CircleAlert;
                return (
                  <li className="flex items-start gap-2 text-xs" key={check.id}>
                    <Icon
                      className={cn(
                        "mt-0.5 size-3.5 shrink-0",
                        healthy
                          ? "text-emerald-600 dark:text-emerald-400"
                          : blocked
                            ? "text-muted-foreground"
                            : "text-destructive",
                      )}
                    />
                    <div className="min-w-0">
                      <div className="font-medium text-foreground">
                        {check.label}
                        <span className="ml-1.5 font-normal text-muted-foreground">
                          {check.status === "willCreate"
                            ? "Will be created"
                            : check.status}
                        </span>
                      </div>
                      {check.detail ? (
                        <p className="text-muted-foreground">{check.detail}</p>
                      ) : null}
                      {check.path ? (
                        <p className="truncate font-mono text-2xs text-muted-foreground">
                          {check.path}
                        </p>
                      ) : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
          {setupQuery.error instanceof Error ? (
            <p className="text-xs text-destructive">
              Setup check failed: {setupQuery.error.message}
            </p>
          ) : null}
        </div>
      ) : null}

      {!workspace ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Choose an Agent workspace before enabling Nxtlinq.
        </p>
      ) : null}
      {!configQuery.isLoading && (!trustStore || !receiptDirectory) ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Configure the shared trust store in Settings → Agents → Nxtlinq
          authorization.
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">
        This Agent inherits the shared trusted signers and receives its own
        receipt directory. Buzz never creates or stores the policy signing key.
      </p>
      {gatewayQuery.data?.available && currentSetup?.ready === false ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Enable is locked until the project owner signs the manifest and the
          deployment operator provisions the external trust store. For this
          repository's demo, run `npm run demo:buzz-project:setup` from the
          Gateway package.
        </p>
      ) : null}
      {failure ? <p className="text-xs text-destructive">{failure}</p> : null}
      {saveBlocked ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Recheck the changed Nxtlinq setup before saving this Agent.
        </p>
      ) : null}
    </div>
  );
}
