import { AlertTriangle, ChevronDown } from "lucide-react";
import * as React from "react";

import {
  useAcpProvidersQuery,
  useAvailableAcpProviders,
  useBackendProvidersQuery,
  useCreateManagedAgentMutation,
  useManagedAgentPrereqsQuery,
} from "@/features/agents/hooks";
import { probeBackendProvider } from "@/shared/api/tauri";
import type {
  BackendProviderProbeResult,
  CreateManagedAgentInput,
  CreateManagedAgentResponse,
  RespondToMode,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  CreateAgentBasicsFields,
  CreateAgentOptionToggles,
  CreateAgentRuntimeProviderField,
  CreateAgentRuntimeFields,
} from "./CreateAgentDialogSections";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  coerceConfigValues,
  ProviderConfigFields,
} from "./ProviderConfigFields";
import { CreateAgentRespondToField } from "./RespondToField";
import { useLastRuntimeProvider } from "@/features/agents/lib/useLastRuntimeProvider";

// ── Dialog ────────────────────────────────────────────────────────────────────

export function CreateAgentDialog({
  open,
  onCreated,
  onOpenChange,
}: {
  open: boolean;
  onCreated: (result: CreateManagedAgentResponse) => void;
  onOpenChange: (open: boolean) => void;
}) {
  const createMutation = useCreateManagedAgentMutation();
  const providersQuery = useAvailableAcpProviders();
  const allProvidersQuery = useAcpProvidersQuery();
  const backendProvidersQuery = useBackendProvidersQuery();
  const { lastProviderId, setLastProvider } = useLastRuntimeProvider();
  const [acpCommand, setAcpCommand] = React.useState("sprout-acp");
  const [agentCommand, setAgentCommand] = React.useState("goose");
  const [agentArgs, setAgentArgs] = React.useState("acp");
  const [mcpCommand, setMcpCommand] = React.useState("sprout-mcp-server");
  const [mcpToolsets, setMcpToolsets] = React.useState("");
  const prereqsQuery = useManagedAgentPrereqsQuery(acpCommand, mcpCommand);
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [spawnAfterCreate, setSpawnAfterCreate] = React.useState(true);
  const [startOnAppLaunch, setStartOnAppLaunch] = React.useState(true);
  const [turnTimeoutSeconds, setTurnTimeoutSeconds] = React.useState("320");
  const [parallelism, setParallelism] = React.useState("24");
  const [systemPrompt, setSystemPrompt] = React.useState("");
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>({});
  const [selectedProviderId, setSelectedProviderId] =
    React.useState<string>("custom");
  const [hasSyncedProviderSelection, setHasSyncedProviderSelection] =
    React.useState(false);
  const [showAdvanced, setShowAdvanced] = React.useState(false);
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    [],
  );

  // ── Backend provider ("Run on") state ──────────────────────────────────────
  const [runOn, setRunOn] = React.useState<"local" | string>("local");
  const [providerConfig, setProviderConfig] = React.useState<
    Record<string, string>
  >({});
  const [probedProvider, setProbedProvider] =
    React.useState<BackendProviderProbeResult | null>(null);
  const [probeError, setProbeError] = React.useState<string | null>(null);

  const providers = providersQuery.data ?? [];
  const allProviders = allProvidersQuery.data ?? [];
  const unavailableCount = allProviders.filter(
    (p) => p.availability !== "available",
  ).length;
  const backendProviders = backendProvidersQuery.data ?? [];
  const prereqs = prereqsQuery.data ?? null;
  const selectedProvider = React.useMemo(
    () =>
      providers.find((provider) => provider.id === selectedProviderId) ?? null,
    [providers, selectedProviderId],
  );
  const selectedBackendProvider = React.useMemo(
    () => backendProviders.find((p) => p.id === runOn) ?? null,
    [backendProviders, runOn],
  );
  const isProviderMode = runOn !== "local";

  const isSpawnSupported =
    prereqs?.acp.available === true && prereqs?.mcp.available === true;
  const spawnToggleDisabled =
    prereqsQuery.isLoading || (prereqs !== null && !isSpawnSupported);
  const isDiscoveryPending = providersQuery.isLoading || prereqsQuery.isLoading;

  React.useEffect(() => {
    if (hasSyncedProviderSelection || providersQuery.isLoading) {
      return;
    }

    // Prefer last-used provider from localStorage
    const remembered = lastProviderId
      ? providers.find((provider) => provider.id === lastProviderId)
      : null;
    if (remembered) {
      setSelectedProviderId(remembered.id);
      setAgentCommand(remembered.command);
      setAgentArgs(remembered.defaultArgs.join(","));
      setMcpCommand(remembered.mcpCommand ?? "sprout-mcp-server");
    } else {
      const matchingProvider =
        providers.find((provider) => provider.command === agentCommand) ?? null;
      if (matchingProvider) {
        setSelectedProviderId(matchingProvider.id);
      }
    }
    setHasSyncedProviderSelection(true);
  }, [
    agentCommand,
    hasSyncedProviderSelection,
    lastProviderId,
    providers,
    providersQuery.isLoading,
  ]);

  React.useEffect(() => {
    if (
      !prereqs ||
      (prereqs.acp.available && prereqs.mcp.available) ||
      !spawnAfterCreate
    ) {
      return;
    }

    setSpawnAfterCreate(false);
  }, [prereqs, spawnAfterCreate]);

  React.useEffect(() => {
    if (
      providersQuery.error instanceof Error ||
      prereqsQuery.error instanceof Error
    ) {
      setShowAdvanced(true);
    }
  }, [prereqsQuery.error, providersQuery.error]);

  // Probe the backend provider when runOn changes to a non-local value
  React.useEffect(() => {
    if (!isProviderMode || !selectedBackendProvider) {
      setProbedProvider(null);
      setProbeError(null);
      return;
    }

    let cancelled = false;
    setProbeError(null);
    setProbedProvider(null);

    probeBackendProvider(selectedBackendProvider.binaryPath)
      .then((result) => {
        if (!cancelled) {
          setProbedProvider(result);
          // Initialize config from schema defaults so unchanged defaults
          // are included in the submit payload (not silently dropped).
          if (result.config_schema) {
            const props =
              (result.config_schema as Record<string, unknown>)?.properties ??
              {};
            const defaults: Record<string, string> = {};
            for (const [key, prop] of Object.entries(props) as [
              string,
              Record<string, unknown>,
            ][]) {
              if (prop.default != null) {
                defaults[key] = String(prop.default);
              }
            }
            setProviderConfig(defaults);
          }
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setProbeError(err instanceof Error ? err.message : String(err));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [isProviderMode, selectedBackendProvider]);

  function reset() {
    setName("");
    setRelayUrl("");
    setSpawnAfterCreate(true);
    setStartOnAppLaunch(true);
    setAcpCommand("sprout-acp");
    setAgentCommand("goose");
    setAgentArgs("acp");
    setMcpCommand("sprout-mcp-server");
    setMcpToolsets("");
    setTurnTimeoutSeconds("320");
    setParallelism("24");
    setSystemPrompt("");
    setEnvVars({});
    setSelectedProviderId("custom");
    setHasSyncedProviderSelection(false);
    setShowAdvanced(false);
    setRunOn("local");
    setProviderConfig({});
    setProbedProvider(null);
    setProbeError(null);
    setRespondTo("owner-only");
    setRespondToAllowlist([]);
    createMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      reset();
    }

    onOpenChange(next);
  }

  function handleProviderChange(nextProviderId: string) {
    setSelectedProviderId(nextProviderId);

    if (nextProviderId === "custom") {
      setShowAdvanced(true);
      return;
    }

    const provider = providers.find(
      (candidate) => candidate.id === nextProviderId,
    );
    if (!provider) {
      return;
    }

    setLastProvider(nextProviderId);
    setAgentCommand(provider.command);
    setAgentArgs(provider.defaultArgs.join(","));
    setMcpCommand(provider.mcpCommand ?? "sprout-mcp-server");
  }

  function handleRunOnChange(value: string) {
    setRunOn(value);
    setProviderConfig({});
    setProbedProvider(null);
    setProbeError(null);
  }

  // Check provider config required fields are filled.
  const providerConfigComplete = React.useMemo(() => {
    if (!isProviderMode || !probedProvider?.config_schema) return true;
    const schema = probedProvider.config_schema as Record<string, unknown>;
    const required: string[] = (schema?.required as string[] | undefined) ?? [];
    return required.every(
      (key) => (providerConfig[key] ?? "").trim().length > 0,
    );
  }, [isProviderMode, probedProvider, providerConfig]);

  // Allowlist mode requires at least one entry, mirroring the harness's own
  // validation. If we let it through empty, the agent crash-loops at startup
  // with a config error.
  const respondToValid =
    respondTo !== "allowlist" || respondToAllowlist.length > 0;

  const canSubmit =
    name.trim().length > 0 &&
    !isDiscoveryPending &&
    !(
      !isProviderMode &&
      spawnAfterCreate &&
      prereqs !== null &&
      !isSpawnSupported
    ) &&
    // Block submission until probe succeeds in provider mode — required
    // fields and config schema are only known after a successful probe.
    !(isProviderMode && !probedProvider) &&
    providerConfigComplete &&
    respondToValid &&
    !createMutation.isPending;

  async function handleSubmit() {
    try {
      // Only send the allowlist when the mode is actually "allowlist".
      // Other modes ignore it server-side, but keeping the wire clean makes
      // the agent record easier to inspect.
      const respondToFields = {
        respondTo,
        respondToAllowlist:
          respondTo === "allowlist" ? respondToAllowlist : undefined,
      } as const;

      const input: CreateManagedAgentInput = isProviderMode
        ? {
            name: name.trim(),
            relayUrl: relayUrl.trim() || undefined,
            turnTimeoutSeconds:
              Number.parseInt(turnTimeoutSeconds, 10) > 0
                ? Number.parseInt(turnTimeoutSeconds, 10)
                : undefined,
            parallelism:
              Number.parseInt(parallelism, 10) > 0
                ? Number.parseInt(parallelism, 10)
                : undefined,
            systemPrompt: systemPrompt.trim() || undefined,
            envVars,
            spawnAfterCreate: true,
            startOnAppLaunch: false, // Remote agents don't auto-start with the desktop
            backend: {
              type: "provider",
              id: runOn,
              config: coerceConfigValues(
                providerConfig,
                probedProvider?.config_schema,
              ),
            },
            ...respondToFields,
          }
        : {
            name: name.trim(),
            relayUrl: relayUrl.trim() || undefined,
            acpCommand: acpCommand.trim() || undefined,
            agentCommand: agentCommand.trim() || undefined,
            agentArgs: agentArgs
              .split(",")
              .map((value) => value.trim())
              .filter((value) => value.length > 0),
            mcpCommand: mcpCommand.trim() || undefined,
            mcpToolsets: mcpToolsets.trim() || undefined,
            turnTimeoutSeconds:
              Number.parseInt(turnTimeoutSeconds, 10) > 0
                ? Number.parseInt(turnTimeoutSeconds, 10)
                : undefined,
            parallelism:
              Number.parseInt(parallelism, 10) > 0
                ? Number.parseInt(parallelism, 10)
                : undefined,
            systemPrompt: systemPrompt.trim() || undefined,
            envVars,
            spawnAfterCreate,
            startOnAppLaunch,
            backend: { type: "local" },
            ...respondToFields,
          };

      const created = await createMutation.mutateAsync(input);
      handleOpenChange(false);
      onCreated(created);
    } catch {
      // React Query stores the error; keep the dialog open and render it inline.
    }
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-3xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Create agent</DialogTitle>
            <DialogDescription>
              This creates a local agent identity, syncs its display name when
              possible, and can spawn `sprout-acp` immediately.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            <CreateAgentBasicsFields name={name} onNameChange={setName} />

            {/* Run on selector — only shown when backend providers are discovered */}
            {backendProviders.length > 0 ? (
              <div className="space-y-1.5">
                <label className="text-sm font-medium" htmlFor="agent-run-on">
                  Run on
                </label>
                <select
                  className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                  id="agent-run-on"
                  onChange={(e) => handleRunOnChange(e.target.value)}
                  value={runOn}
                >
                  <option value="local">This computer</option>
                  {backendProviders.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.id}
                    </option>
                  ))}
                </select>
              </div>
            ) : null}

            {/* Provider mode: trust warning + config fields */}
            {isProviderMode && selectedBackendProvider ? (
              <div className="space-y-4">
                <div className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3">
                  <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
                  <p className="text-sm text-warning">
                    This provider at{" "}
                    <span className="font-mono font-medium">
                      {selectedBackendProvider.binaryPath}
                    </span>{" "}
                    will receive your agent&apos;s private key. Only use
                    providers from trusted sources.
                  </p>
                </div>

                {probeError ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    Could not probe provider: {probeError}
                  </p>
                ) : null}

                {probedProvider?.config_schema ? (
                  <ProviderConfigFields
                    config={providerConfig}
                    onChange={setProviderConfig}
                    schema={probedProvider.config_schema}
                  />
                ) : null}
              </div>
            ) : null}

            {/* Local mode: show the ACP runtime selector */}
            {!isProviderMode ? (
              <CreateAgentRuntimeProviderField
                onProviderChange={handleProviderChange}
                providers={providers}
                providersLoading={providersQuery.isLoading}
                selectedProvider={selectedProvider}
                selectedProviderId={selectedProviderId}
                unavailableCount={unavailableCount}
              />
            ) : null}

            <CreateAgentOptionToggles
              isSpawnSupported={isSpawnSupported}
              onToggleStartOnAppLaunch={() => {
                setStartOnAppLaunch((current) => !current);
              }}
              onToggleSpawnAfterCreate={() => {
                if (!spawnToggleDisabled) {
                  setSpawnAfterCreate((current) => !current);
                }
              }}
              prereqs={prereqs}
              startOnAppLaunch={isProviderMode ? false : startOnAppLaunch}
              startOnAppLaunchDisabled={isProviderMode}
              spawnAfterCreate={isProviderMode ? true : spawnAfterCreate}
              spawnToggleDisabled={isProviderMode || spawnToggleDisabled}
            />

            <CreateAgentRespondToField
              allowlist={respondToAllowlist}
              mode={respondTo}
              onAllowlistChange={setRespondToAllowlist}
              onModeChange={setRespondTo}
            />

            <div className="rounded-2xl border border-border/70 bg-muted/20">
              <button
                aria-expanded={showAdvanced}
                className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left"
                onClick={() => setShowAdvanced((current) => !current)}
                type="button"
              >
                <div>
                  <p className="text-sm font-semibold tracking-tight">
                    Advanced setup
                  </p>
                  <p className="text-sm text-muted-foreground">
                    Relay overrides, raw commands, timeout, parallelism, prompt
                    override, and doctor guidance.
                  </p>
                </div>
                <span className="shrink-0 self-center text-muted-foreground">
                  <ChevronDown
                    className={`h-4 w-4 transition-transform ${showAdvanced ? "rotate-180" : ""}`}
                  />
                </span>
              </button>

              {showAdvanced ? (
                <div className="overflow-hidden">
                  <div className="space-y-5 border-t border-border/60 px-4 py-4">
                    <CreateAgentRuntimeFields
                      acpCommand={acpCommand}
                      agentArgs={agentArgs}
                      agentCommand={agentCommand}
                      mcpCommand={mcpCommand}
                      mcpToolsets={mcpToolsets}
                      onParallelismChange={setParallelism}
                      onAcpCommandChange={setAcpCommand}
                      onAgentArgsChange={setAgentArgs}
                      onAgentCommandChange={setAgentCommand}
                      onMcpCommandChange={setMcpCommand}
                      onMcpToolsetsChange={setMcpToolsets}
                      onRelayUrlChange={setRelayUrl}
                      onSystemPromptChange={setSystemPrompt}
                      onTurnTimeoutChange={setTurnTimeoutSeconds}
                      parallelism={parallelism}
                      relayUrl={relayUrl}
                      selectedProviderId={selectedProviderId}
                      systemPrompt={systemPrompt}
                      turnTimeoutSeconds={turnTimeoutSeconds}
                    />

                    <p className="rounded-2xl border border-border/70 bg-background/70 px-4 py-3 text-sm text-muted-foreground">
                      Local Sprout binary checks and ACP runtime discovery now
                      live in Settings &gt; Doctor.
                    </p>

                    {providersQuery.error instanceof Error ? (
                      <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                        {providersQuery.error.message}
                      </p>
                    ) : null}

                    {prereqsQuery.error instanceof Error ? (
                      <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                        {prereqsQuery.error.message}
                      </p>
                    ) : null}
                  </div>
                </div>
              ) : null}
            </div>

            <EnvVarsEditor
              disabled={createMutation.isPending}
              helperText="Injected at spawn. Overrides the persona's env vars on collision."
              onChange={setEnvVars}
              value={envVars}
            />

            {createMutation.error instanceof Error ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {createMutation.error.message}
              </p>
            ) : null}
          </div>

          <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
            <Button
              onClick={() => handleOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              data-testid="create-agent-submit"
              disabled={!canSubmit}
              onClick={() => void handleSubmit()}
              size="sm"
              type="button"
            >
              {createMutation.isPending ? "Creating..." : "Create agent"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
