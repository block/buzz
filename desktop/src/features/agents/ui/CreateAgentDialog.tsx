import { AlertTriangle, ChevronDown, Loader2 } from "lucide-react";
import * as React from "react";

import {
  useAcpRuntimesQuery,
  useAvailableAcpRuntimes,
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
import { Textarea } from "@/shared/ui/textarea";
import type { AgentDraft } from "./agentDraft";
import { AgentCreationPreview } from "./AgentCreationPreview";
import { AgentModelProviderFields } from "./AgentModelProviderFields";
import {
  CreateAgentBasicsFields,
  CreateAgentOptionToggles,
  CreateAgentRuntimeField,
  CreateAgentRuntimeFields,
} from "./CreateAgentDialogSections";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import { providerRequiresExplicitModel } from "./personaDialogPickers";
import {
  coerceConfigValues,
  ProviderConfigFields,
} from "./ProviderConfigFields";
import { CreateAgentRespondToField } from "./RespondToField";
import { RelayMeshAgentSection } from "@/features/mesh-compute/ui/RelayMeshAgentSection";
import { meshPrepareRelayMeshClient } from "@/shared/api/tauriMesh";
import type { MeshServeTarget } from "@/shared/api/tauriMesh";
import { useLastRuntime } from "@/features/agents/lib/useLastRuntime";

export function CreateAgentDialog({
  open,
  draft,
  onCreated,
  onOpenChange,
}: {
  open: boolean;
  /** Prefilled starting point (blank / template / import / duplicate). */
  draft: AgentDraft | null;
  onCreated: (result: CreateManagedAgentResponse) => void;
  onOpenChange: (open: boolean) => void;
}) {
  const createMutation = useCreateManagedAgentMutation();
  const providersQuery = useAvailableAcpRuntimes({ enabled: open });
  const allProvidersQuery = useAcpRuntimesQuery({ enabled: open });
  const backendProvidersQuery = useBackendProvidersQuery({ enabled: open });
  const { lastRuntimeId, setLastRuntime } = useLastRuntime();
  const [acpCommand, setAcpCommand] = React.useState("buzz-acp");
  const [agentCommand, setAgentCommand] = React.useState("buzz-agent");
  const [agentArgs, setAgentArgs] = React.useState("acp");
  const [mcpCommand, setMcpCommand] = React.useState("");
  const [mcpToolsets, setMcpToolsets] = React.useState("");
  const prereqsQuery = useManagedAgentPrereqsQuery(acpCommand, mcpCommand, {
    enabled: open,
  });
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [spawnAfterCreate, setSpawnAfterCreate] = React.useState(true);
  const [startOnAppLaunch, setStartOnAppLaunch] = React.useState(true);
  const [turnTimeoutSeconds, setTurnTimeoutSeconds] = React.useState("320");
  const [parallelism, setParallelism] = React.useState("24");
  const [systemPrompt, setSystemPrompt] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const [model, setModel] = React.useState("");
  const [llmProvider, setLlmProvider] = React.useState("");
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>({});
  const [selectedRuntimeId, setSelectedRuntimeId] =
    React.useState<string>("custom");
  const [hasSyncedProviderSelection, setHasSyncedProviderSelection] =
    React.useState(false);
  const [hasAppliedDraft, setHasAppliedDraft] = React.useState(false);
  const [showAdvanced, setShowAdvanced] = React.useState(false);
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    [],
  );

  const [runOn, setRunOn] = React.useState<"local" | string>("local");
  const [providerConfig, setProviderConfig] = React.useState<
    Record<string, string>
  >({});
  const [probedProvider, setProbedProvider] =
    React.useState<BackendProviderProbeResult | null>(null);
  const [probeError, setProbeError] = React.useState<string | null>(null);

  // When `useMesh` is on, the agent runs buzz-agent against a member's
  // shared compute. The ACP runtime + backend selectors are hidden; runtime
  // fields are driven by `mesh_agent_preset(meshModelId)` and the submit
  // input carries `model: meshModelId`.
  const [useMesh, setUseMesh] = React.useState(false);
  const [meshModelId, setMeshModelId] = React.useState("");
  const [meshTarget, setMeshTarget] = React.useState<MeshServeTarget | null>(
    null,
  );
  const [meshClientError, setMeshClientError] = React.useState<string | null>(
    null,
  );
  // True while the relay-mesh client preflight (connect-request + dial) runs
  // inside handleSubmit. That await can take tens of seconds and happens
  // before createMutation fires, so isPending alone leaves the button live.
  const [meshPreparing, setMeshPreparing] = React.useState(false);

  const runtimes = providersQuery.data ?? [];
  const allProviders = allProvidersQuery.data ?? [];
  const unavailableCount = allProviders.filter(
    (p) => p.availability !== "available",
  ).length;
  const backendProviders = backendProvidersQuery.data ?? [];
  const prereqs = prereqsQuery.data ?? null;
  const selectedRuntime = React.useMemo(
    () => runtimes.find((runtime) => runtime.id === selectedRuntimeId) ?? null,
    [runtimes, selectedRuntimeId],
  );
  const selectedBackendProvider = React.useMemo(
    () => backendProviders.find((p) => p.id === runOn) ?? null,
    [backendProviders, runOn],
  );
  // Relay mesh always runs in local mode (buzz-agent + OpenAI-compat env);
  // when on, it suppresses the backend "Run on" branch even if a stale
  // `runOn` value remains. The relay-mesh path is its own thing.
  const isProviderMode = !useMesh && runOn !== "local";

  const isSpawnSupported =
    prereqs?.acp.available === true && prereqs?.mcp.available === true;
  const spawnToggleDisabled =
    prereqsQuery.isLoading || (prereqs !== null && !isSpawnSupported);
  const isDiscoveryPending = providersQuery.isLoading || prereqsQuery.isLoading;

  // Apply the draft's simple fields once per open. Runtime selection happens
  // separately below because it needs the runtime catalog to resolve.
  React.useEffect(() => {
    if (!open || !draft || hasAppliedDraft) {
      return;
    }

    setName(draft.name);
    setSystemPrompt(draft.systemPrompt);
    setAvatarUrl(draft.avatarUrl);
    setModel(draft.model ?? "");
    setLlmProvider(draft.provider ?? "");
    setEnvVars({ ...draft.envVars });
    setHasAppliedDraft(true);
  }, [draft, hasAppliedDraft, open]);

  React.useEffect(() => {
    if (hasSyncedProviderSelection || providersQuery.isLoading) {
      return;
    }

    // Prefer the draft's runtime hint (an id like "goose" or a resolved
    // command for duplicated agents), then the last-used runtime.
    const draftRuntime = draft?.runtime
      ? (runtimes.find((runtime) => runtime.id === draft.runtime) ??
        runtimes.find((runtime) => runtime.command === draft.runtime) ??
        null)
      : null;
    const remembered =
      draftRuntime ??
      (lastRuntimeId
        ? runtimes.find((runtime) => runtime.id === lastRuntimeId)
        : null);
    if (remembered) {
      setSelectedRuntimeId(remembered.id);
      setAgentCommand(remembered.command);
      setAgentArgs(remembered.defaultArgs.join(","));
      setMcpCommand(remembered.mcpCommand ?? "");
    } else {
      const matchingProvider =
        runtimes.find((runtime) => runtime.command === agentCommand) ?? null;
      if (matchingProvider) {
        setSelectedRuntimeId(matchingProvider.id);
      }
    }
    setHasSyncedProviderSelection(true);
  }, [
    agentCommand,
    draft,
    hasSyncedProviderSelection,
    lastRuntimeId,
    runtimes,
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
    setAcpCommand("buzz-acp");
    setAgentCommand("buzz-agent");
    setAgentArgs("acp");
    setMcpCommand("");
    setMcpToolsets("");
    setTurnTimeoutSeconds("320");
    setParallelism("24");
    setSystemPrompt("");
    setAvatarUrl("");
    setIsAvatarUploadPending(false);
    setModel("");
    setLlmProvider("");
    setEnvVars({});
    setSelectedRuntimeId("custom");
    setHasSyncedProviderSelection(false);
    setHasAppliedDraft(false);
    setShowAdvanced(false);
    setRunOn("local");
    setProviderConfig({});
    setProbedProvider(null);
    setProbeError(null);
    setUseMesh(false);
    setMeshModelId("");
    setMeshClientError(null);
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
    setSelectedRuntimeId(nextProviderId);

    if (nextProviderId === "custom") {
      setShowAdvanced(true);
      return;
    }

    const provider = runtimes.find(
      (candidate) => candidate.id === nextProviderId,
    );
    if (!provider) {
      return;
    }

    setLastRuntime(nextProviderId);
    setAgentCommand(provider.command);
    setAgentArgs(provider.defaultArgs.join(","));
    setMcpCommand(provider.mcpCommand ?? "");
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

  const showModelProviderFields =
    !isProviderMode && !useMesh && selectedRuntimeId !== "custom";
  // Anthropic/OpenAI-style providers have no server-side default model — an
  // empty model would crash the harness at startup.
  const modelRequirementSatisfied =
    !showModelProviderFields ||
    !providerRequiresExplicitModel(llmProvider) ||
    model.trim().length > 0;

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
    // Relay-mesh mode requires a concrete serve target, not just a model name.
    !(useMesh && (meshModelId.trim().length === 0 || meshTarget == null)) &&
    respondToValid &&
    modelRequirementSatisfied &&
    !isAvatarUploadPending &&
    !meshPreparing &&
    !createMutation.isPending;

  async function handleSubmit() {
    setMeshClientError(null);
    try {
      if (useMesh) {
        try {
          if (!meshTarget) {
            setMeshClientError(
              "Select a relay mesh serve target before creating the agent.",
            );
            return;
          }
          setMeshPreparing(true);
          try {
            await meshPrepareRelayMeshClient(meshModelId.trim(), meshTarget);
          } finally {
            setMeshPreparing(false);
          }
        } catch (err) {
          setMeshClientError(err instanceof Error ? err.message : String(err));
          return;
        }
      }

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
            avatarUrl: avatarUrl.trim() || undefined,
            model: model.trim() || undefined,
            provider: llmProvider.trim() || undefined,
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
            avatarUrl: avatarUrl.trim() || undefined,
            envVars,
            model: useMesh
              ? meshModelId.trim() || undefined
              : model.trim() || undefined,
            provider: useMesh ? undefined : llmProvider.trim() || undefined,
            relayMesh: useMesh ? { modelRef: meshModelId.trim() } : undefined,
            spawnAfterCreate,
            // Relay-mesh agents need a freshly selected serve target to start;
            // do not auto-restore them later with only the saved model/env.
            startOnAppLaunch: useMesh ? false : startOnAppLaunch,
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
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && (createMutation.isPending || isAvatarUploadPending)) {
          return;
        }
        handleOpenChange(nextOpen);
      }}
      open={open}
    >
      <DialogContent className="max-w-3xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>{draft?.title ?? "Create agent"}</DialogTitle>
            <DialogDescription>
              {draft?.description ??
                "Set up a new agent and start it in this workspace."}
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            <div className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]">
              <AgentCreationPreview
                avatarUrl={avatarUrl.trim() || null}
                disabled={createMutation.isPending || isAvatarUploadPending}
                label={name.trim() || "Agent name"}
                onClearAvatar={() => setAvatarUrl("")}
                onSelectAvatar={setAvatarUrl}
                onUploadPendingChange={setIsAvatarUploadPending}
              />

              <div className="space-y-5">
                <CreateAgentBasicsFields name={name} onNameChange={setName} />

                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium"
                    htmlFor="agent-instructions"
                  >
                    Agent instructions
                  </label>
                  <Textarea
                    className="min-h-32 resize-y"
                    data-testid="agent-instructions-input"
                    id="agent-instructions"
                    onChange={(event) => setSystemPrompt(event.target.value)}
                    placeholder="Describe what this agent should do."
                    value={systemPrompt}
                  />
                </div>

                <RelayMeshAgentSection
                  current={{
                    acpCommand,
                    agentCommand,
                    agentArgs: agentArgs
                      .split(",")
                      .map((v) => v.trim())
                      .filter((v) => v.length > 0),
                    mcpCommand,
                    model: meshModelId || null,
                    envVars,
                  }}
                  modelId={meshModelId}
                  targetEndpointAddr={meshTarget?.endpointAddr ?? ""}
                  onModelIdChange={(nextId, patch) => {
                    setMeshModelId(nextId);
                    if (patch == null) return;
                    // Fan out the preset into the existing setters so the rest
                    // of the dialog (and the submit branch) see normal local-mode
                    // values — relay-mesh is a curated local agent.
                    setAcpCommand(patch.acpCommand);
                    setAgentCommand(patch.agentCommand);
                    setAgentArgs(patch.agentArgs.join(","));
                    setMcpCommand(patch.mcpCommand);
                    setEnvVars(patch.envVars);
                  }}
                  onTargetChange={setMeshTarget}
                  onUseMeshChange={(next) => {
                    setUseMesh(next);
                    if (!next) {
                      // Clearing the toggle: drop the model selection so the
                      // submit guard doesn't fire on a stale value. The runtime
                      // fields keep whatever the user had — they can re-pick
                      // ACP runtime or stay with the preset values, their call.
                      setMeshModelId("");
                      setMeshTarget(null);
                    }
                  }}
                  useMesh={useMesh}
                />

                {/* Run on selector — only shown when backend providers are discovered */}
                {!useMesh && backendProviders.length > 0 ? (
                  <div className="space-y-1.5">
                    <label
                      className="text-sm font-medium"
                      htmlFor="agent-run-on"
                    >
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
                {!isProviderMode && !useMesh ? (
                  <CreateAgentRuntimeField
                    onRuntimeChange={handleProviderChange}
                    runtimes={runtimes}
                    runtimesLoading={providersQuery.isLoading}
                    selectedRuntime={selectedRuntime}
                    selectedRuntimeId={selectedRuntimeId}
                    unavailableCount={unavailableCount}
                  />
                ) : null}

                {showModelProviderFields ? (
                  <AgentModelProviderFields
                    disabled={createMutation.isPending}
                    envVars={envVars}
                    model={model}
                    onEnvVarsChange={setEnvVars}
                    onModelChange={setModel}
                    onProviderChange={setLlmProvider}
                    open={open}
                    provider={llmProvider}
                    runtimeId={selectedRuntimeId}
                    selectedRuntime={selectedRuntime ?? undefined}
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
                        Relay overrides, raw commands, timeout, parallelism, and
                        doctor guidance.
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
                          onTurnTimeoutChange={setTurnTimeoutSeconds}
                          parallelism={parallelism}
                          relayUrl={relayUrl}
                          selectedRuntimeId={selectedRuntimeId}
                          turnTimeoutSeconds={turnTimeoutSeconds}
                        />

                        <p className="rounded-2xl border border-border/70 bg-background/70 px-4 py-3 text-sm text-muted-foreground">
                          Local Buzz binary checks and ACP runtime discovery now
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
                  helperText="Injected when the agent process starts."
                  onChange={setEnvVars}
                  value={envVars}
                />

                {meshClientError ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    Mesh client failed to start: {meshClientError}
                  </p>
                ) : null}

                {createMutation.error instanceof Error ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {createMutation.error.message}
                  </p>
                ) : null}
              </div>
            </div>
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
              {isDiscoveryPending ? (
                <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
              ) : null}
              {meshPreparing
                ? "Connecting to mesh..."
                : createMutation.isPending
                  ? "Creating..."
                  : isAvatarUploadPending
                    ? "Uploading..."
                    : (draft?.submitLabel ?? "Create agent")}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
