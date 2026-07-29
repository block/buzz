import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, ExternalLink, Loader2 } from "lucide-react";
import * as React from "react";

import {
  useBackendProvidersQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { NO_BACKEND_PROVIDER_HINT } from "@/features/agents/lib/backendProviderLabel";
import { addedExclusiveHarnessIds } from "@/features/agents/lib/exclusiveRemoteHarness";
import { REMOTE_TEAM_INSTRUCTIONS_NOTICE } from "@/features/agents/lib/remoteTeamInstructions";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import {
  discoverProviderHarnesses,
  probeBackendProvider,
  probeProviderModels,
} from "@/shared/api/tauri";
import type { ProviderRecovery } from "@/shared/api/tauri";
import type { ManagedAgent, RemoteHarness } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

import type { EnvVarsValue } from "./EnvVarsEditor";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  coerceConfigValues,
  ProviderConfigFields,
} from "./ProviderConfigFields";
import {
  autoPickRemoteHarness,
  emptyWhereToRunDraft,
  type HostFailure,
  hostFailureOf,
  LOCAL_RUN_TARGET_VALUE,
  providerConfigComplete,
  rememberProbedProviderName,
  remoteHarnessOptions,
  runTargetOptions,
  type WhereToRunDraft,
} from "./whereToRunIntent";

/**
 * The create flow's first question: which computer this agent runs on, and —
 * once that answer is a backend provider — everything about that host.
 *
 * It leads the dialog because it is the one answer the rest of the form
 * depends on: the harness comes from the host's catalog and the models come
 * from the host's harness, so asking it last means answering the dependent
 * questions against the wrong machine first. Buzz shared compute is an LLM
 * provider, not a run destination, so it is not a choice here.
 */
export function WhereToRunSection({
  draft,
  envVars,
  isPending,
  onDraftChange,
}: {
  draft: WhereToRunDraft;
  /**
   * The definition's credential env, forwarded to the host's model probe. A
   * remote harness resolves its catalog from an API key exactly as the local
   * one does, so without these an Anthropic-backed harness would answer with
   * an auth error rather than a model list. Passed in (rather than read here)
   * because it is unsaved dialog state; the global layer beneath it is a
   * shared query, so this component reads that itself.
   */
  envVars: EnvVarsValue;
  isPending: boolean;
  onDraftChange: (next: WhereToRunDraft) => void;
}) {
  const backendProviders = useBackendProvidersQuery().data ?? [];
  // The agents that already exist, so an exclusive catalog entry one of them
  // occupies can be refused. This is the same shared query the Agents surfaces
  // read, so it is warm by the time this dialog opens.
  const managedAgents = useManagedAgentsQuery().data ?? [];
  const { globalConfig } = useGlobalAgentConfig();
  const [probeError, setProbeError] = React.useState<string | null>(null);
  const [harnessError, setHarnessError] = React.useState<HostFailure | null>(
    null,
  );
  const [isDiscoveringHarnesses, setIsDiscoveringHarnesses] =
    React.useState(false);
  // Friendly provider names, accumulated as probes land. Kept here rather than
  // derived from the current selection so a provider keeps one name for the
  // life of the dialog instead of renaming itself as the cursor moves.
  const [probedProviderNames, setProbedProviderNames] = React.useState<
    Readonly<Record<string, string>>
  >({});
  const isProviderMode = draft.runOn !== LOCAL_RUN_TARGET_VALUE;
  const selectedBackendProvider = React.useMemo(
    () =>
      backendProviders.find((provider) => provider.id === draft.runOn) ?? null,
    [backendProviders, draft.runOn],
  );

  // The probe effect writes back into the draft it reads. Reading it through a
  // ref instead of the dependency array is what keeps that from being a
  // self-retriggering loop (probe → onDraftChange → new draft identity →
  // probe): the provider selection is the only thing that should re-probe.
  const draftRef = React.useRef(draft);
  draftRef.current = draft;
  // Read at call time for the same reason the harness catalog is: an env edit
  // must not open an SSH connection per keystroke.
  //
  // Global sits UNDER the definition's env, the same order `provider_deploy`
  // merges on the host. Without the global layer a key satisfied globally —
  // which the dialog then shows as inherited, with no required marker, so the
  // user has no reason to restate it — never reaches the probe, and the host
  // answers the model request with an auth error for a credential that is in
  // fact configured.
  const probeEnvRef = React.useRef<EnvVarsValue>({});
  probeEnvRef.current = { ...globalConfig.env_vars, ...envVars };
  // Read at call time so the exclusivity check judges the catalog against the
  // agents that exist when the answer LANDS, not when the button was pressed —
  // a create in another window during the SSH round trip is exactly the race
  // this guard exists for.
  const agentsRef = React.useRef<readonly ManagedAgent[]>(managedAgents);
  agentsRef.current = managedAgents;
  // Serial number of the newest host request. Every catalog read and model
  // probe claims one at its start and re-checks it after each await; anything
  // that moves the draft off the host/harness a request was made for bumps it,
  // so the stale continuation drops its answer instead of writing it back.
  const hostRequestRef = React.useRef(0);

  React.useEffect(() => {
    if (!isProviderMode || !selectedBackendProvider) {
      setProbeError(null);
      return;
    }
    let cancelled = false;
    setProbeError(null);
    void probeBackendProvider(selectedBackendProvider.binaryPath)
      .then((result) => {
        if (cancelled) return;
        setProbedProviderNames((previous) =>
          rememberProbedProviderName(
            previous,
            selectedBackendProvider.id,
            result,
          ),
        );
        const defaults: Record<string, string> = {};
        const properties =
          (result.config_schema as Record<string, unknown> | undefined)
            ?.properties ?? {};
        for (const [key, property] of Object.entries(properties) as [
          string,
          Record<string, unknown>,
        ][]) {
          if (property.default != null)
            defaults[key] = String(property.default);
        }
        onDraftChange({
          ...draftRef.current,
          probedProvider: result,
          // Schema defaults are seeded UNDERNEATH what the user has typed.
          // The probe is a round-trip to the provider binary and re-runs
          // whenever it resolves anew, so overwriting here would wipe an
          // address mid-edit. A provider switch empties the draft, so on the
          // first probe of a provider this is exactly `defaults`.
          providerConfig: { ...defaults, ...draftRef.current.providerConfig },
        });
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setProbeError(error instanceof Error ? error.message : String(error));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [isProviderMode, onDraftChange, selectedBackendProvider]);

  /**
   * Abandon every in-flight host request. Their answers describe a
   * host/harness the draft no longer targets, so landing one would scope the
   * picker to the wrong machine — or, for a model probe, ship the definition's
   * credentials to a host under a harness command never verified there.
   */
  // Stable identity: an effect below depends on it, and a per-render function
  // there would re-run the effect on every render.
  const discardHostRequests = React.useCallback(() => {
    hostRequestRef.current += 1;
  }, []);

  /** Claim the newest request id, invalidating anything already in flight. */
  function startHostRequest(): number {
    discardHostRequests();
    return hostRequestRef.current;
  }

  // Discovery is an explicit action, not an effect on the config fields: it
  // opens a real SSH connection to the host, which must not happen once per
  // keystroke while the address is being typed.
  async function handleDiscoverHarnesses() {
    if (!selectedBackendProvider) return;
    const requestId = startHostRequest();
    setHarnessError(null);
    setIsDiscoveringHarnesses(true);
    try {
      const config = coerceConfigValues(
        draftRef.current.providerConfig,
        draftRef.current.probedProvider?.config_schema,
      );
      const catalog = await discoverProviderHarnesses(
        selectedBackendProvider.binaryPath,
        config,
      );
      // The config can be edited (or the provider re-picked) while this read
      // is open. That answer then describes an abandoned host: re-installing
      // its catalog would resurrect a pin the edit deliberately cleared, and
      // the probe below would send credentials to the NEW host under the OLD
      // host's harness command. Drop it.
      if (hostRequestRef.current !== requestId) return;
      // Auto-pick skips entries an existing agent already occupies: an
      // exclusive entry is a persistent identity on the host, and arming the
      // create with one the picker itself refuses would ship a second agent
      // onto it on submit. Computed against the config this read used, not the
      // draft's current one.
      const firstAvailable = autoPickRemoteHarness(
        catalog.harnesses,
        addedExclusiveHarnessIds(
          catalog.harnesses,
          { providerId: selectedBackendProvider.id, config },
          agentsRef.current,
        ),
        draftRef.current.remoteHarnessId,
      );
      const next = {
        ...draftRef.current,
        remoteHarnesses: catalog.harnesses,
        remoteHarnessId: firstAvailable?.id ?? null,
        remoteModelProbe: { status: "idle" } as const,
      };
      onDraftChange(next);
      if (!catalog.buzzAcp) {
        // Deploy installs buzz-acp only when this desktop has a binary to
        // push (see docs/remote-agents.md); without one it fails with install
        // guidance. The copy promises the union honestly rather than guessing
        // which case applies from here.
        setHarnessError({
          message:
            "buzz-acp is not installed on that host. Deploy will install it or explain how to.",
          recovery: null,
        });
      }
      // A re-check can change what the auto-picked harness resolves to even
      // when the id is unchanged (a reinstall, a different PATH entry), so the
      // catalog read always re-probes rather than trusting a prior result.
      if (firstAvailable) void probeModels(firstAvailable, next);
    } catch (error: unknown) {
      if (hostRequestRef.current !== requestId) return;
      setHarnessError(hostFailureOf(error));
    } finally {
      // Unconditional: the button is disabled while this flag is set, so no
      // second catalog read can be in flight to own it — and the model probe
      // started just above deliberately claims a newer id. Guarding here would
      // strand "Checking host…" on screen with no way to retry.
      setIsDiscoveringHarnesses(false);
    }
  }

  /**
   * Read the picked harness's model catalog FROM THE HOST.
   *
   * This is the whole point of the remote path: `get_agent_models` /
   * `discover_agent_models` answer for this computer, so a model chosen from
   * their list is validated against the wrong machine — the exact
   * remote/local confusion a provider-backed create exists to avoid.
   *
   * Failure is non-fatal by design. The host's catalog scopes the picker; it
   * does not gate the create, and the harness's own default remains a valid
   * choice when the probe cannot run.
   */
  async function probeModels(harness: RemoteHarness, base: WhereToRunDraft) {
    if (!selectedBackendProvider) return;
    const requestId = startHostRequest();
    // `base` rather than `draftRef.current`: the caller has just published the
    // harness pick, and React has not re-rendered yet, so the ref still holds
    // the pre-pick draft. Spreading it here would revert the pick.
    onDraftChange({ ...base, remoteModelProbe: { status: "loading" } });
    try {
      const models = await probeProviderModels(
        selectedBackendProvider.binaryPath,
        coerceConfigValues(
          base.providerConfig,
          base.probedProvider?.config_schema,
        ),
        harness,
        // The harness's own catalog env rides underneath the user's, so a
        // user-set key wins over a default exactly as it does at spawn.
        { ...harness.env, ...probeEnvRef.current },
      );
      if (hostRequestRef.current !== requestId) return;
      onDraftChange({
        ...draftRef.current,
        remoteModelProbe: { status: "loaded", models },
      });
      // A round trip that reached the host proves the auth problem below is
      // gone, so its "Authenticate in browser" button must not outlive it.
      // Only that failure is cleared: a catalog complaint (a host with no
      // buzz-acp) is still true and is not what this call tested.
      setHarnessError((previous) => (previous?.recovery ? null : previous));
    } catch (error: unknown) {
      if (hostRequestRef.current !== requestId) return;
      const failure = hostFailureOf(error);
      onDraftChange({
        ...draftRef.current,
        remoteModelProbe: { status: "failed", error: failure.message },
      });
      // A model probe that failed on browser auth is the SAME host problem the
      // catalog read reports, and the Model control has no room for an action.
      // Surfacing it on the harness picker keeps one "authenticate, then check
      // the host again" affordance instead of two competing ones.
      if (failure.recovery) setHarnessError(failure);
    }
  }

  function handleSelectHarness(remoteHarnessId: string) {
    discardHostRequests();
    const harness = (draft.remoteHarnesses ?? []).find(
      (candidate) => candidate.id === remoteHarnessId,
    );
    const next = {
      ...draft,
      remoteHarnessId,
      remoteModelProbe: { status: "idle" } as const,
    };
    if (!harness) {
      onDraftChange(next);
      return;
    }
    void probeModels(harness, next);
  }

  // No provider installed is a legitimate answer to this question, not a
  // reason to skip it. As the LAST step the section could vanish silently; as
  // the FIRST one, vanishing would leave the user with no evidence that
  // running elsewhere is even a thing Buzz does — so the control stays and
  // explains why it has only one entry.
  const hasProviders = backendProviders.length > 0;

  // The catalog entries an existing agent already occupies. Recomputed as the
  // agent list refreshes, so an agent created elsewhere disables its entry here
  // without a re-check of the host.
  const addedExclusiveIds = React.useMemo(
    () =>
      addedExclusiveHarnessIds(
        draft.remoteHarnesses ?? [],
        {
          providerId: draft.runOn,
          config: coerceConfigValues(
            draft.providerConfig,
            draft.probedProvider?.config_schema,
          ),
        },
        managedAgents,
      ),
    [
      draft.probedProvider?.config_schema,
      draft.providerConfig,
      draft.remoteHarnesses,
      draft.runOn,
      managedAgents,
    ],
  );

  // A pick can go stale while the dialog is open: another window (or this one,
  // in an earlier create) can take the identity between the catalog read and
  // submit, and the agent list refreshes on its own. The row goes disabled, but
  // the PIN would survive and submit would still deploy the second agent — so
  // the selection is dropped with it. Clearing is safe to repeat: once the id
  // is null the condition cannot hold again.
  const staleHarnessId =
    draft.remoteHarnessId && addedExclusiveIds.has(draft.remoteHarnessId)
      ? draft.remoteHarnessId
      : null;
  React.useEffect(() => {
    if (!staleHarnessId) return;
    discardHostRequests();
    onDraftChange({
      ...draftRef.current,
      remoteHarnessId: null,
      remoteModelProbe: { status: "idle" },
    });
  }, [discardHostRequests, onDraftChange, staleHarnessId]);

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="agent-run-on"
        >
          Where does this agent run?
        </label>
        <PersonaDropdownField
          disabled={isPending || !hasProviders}
          id="agent-run-on"
          onValueChange={(runOn) => {
            discardHostRequests();
            onDraftChange({ ...emptyWhereToRunDraft, runOn });
          }}
          options={runTargetOptions(backendProviders, probedProviderNames)}
          placeholder="This computer"
          value={draft.runOn}
        />
        {!hasProviders ? (
          <p className="text-xs text-muted-foreground">
            {NO_BACKEND_PROVIDER_HINT}
          </p>
        ) : null}
      </div>

      {isProviderMode && selectedBackendProvider ? (
        <div className="space-y-4">
          <div className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              This provider at{" "}
              <span className="font-mono font-medium">
                {selectedBackendProvider.binaryPath}
              </span>{" "}
              will receive your agent&apos;s private key. Only use providers
              from trusted sources.
            </p>
          </div>
          {/* Stated the moment "elsewhere" is the answer, and unconditionally:
              the team is chosen after this section, so waiting for one to be
              picked would surface the limitation only where it is already too
              late to weigh. See `remoteTeamInstructions`. */}
          <p
            className="text-xs text-muted-foreground"
            data-testid="remote-team-instructions-notice"
          >
            {REMOTE_TEAM_INSTRUCTIONS_NOTICE}
          </p>
          {probeError ? (
            <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
              Could not probe provider: {probeError}
            </p>
          ) : null}
          {draft.probedProvider?.config_schema ? (
            <ProviderConfigFields
              config={draft.providerConfig}
              onChange={(providerConfig) => {
                discardHostRequests();
                onDraftChange({
                  ...draft,
                  providerConfig,
                  // Config edits invalidate a catalog read from the previous
                  // host — a stale pin would deploy a command that may not
                  // exist on the new one, and stale models would scope the
                  // picker to a machine the agent is no longer going to.
                  remoteHarnesses: null,
                  remoteHarnessId: null,
                  remoteModelProbe: { status: "idle" },
                });
              }}
              schema={draft.probedProvider.config_schema}
            />
          ) : null}

          <RemoteHarnessPicker
            addedExclusiveIds={addedExclusiveIds}
            draft={draft}
            error={harnessError}
            isDiscovering={isDiscoveringHarnesses}
            isPending={isPending}
            onDiscover={() => void handleDiscoverHarnesses()}
            onSelect={handleSelectHarness}
          />
        </div>
      ) : null}
    </div>
  );
}

/**
 * Harness selection for a remote agent. Deliberately separate from the local
 * runtime picker: that one lists what is installed on THIS computer, which says
 * nothing about the host, and the entry chosen here is what the deploy actually
 * runs there.
 */
function RemoteHarnessPicker({
  addedExclusiveIds,
  draft,
  error,
  isDiscovering,
  isPending,
  onDiscover,
  onSelect,
}: {
  /**
   * Catalog entries that name a persistent identity on the host which an
   * existing agent already drives. Rendered disabled with an "(added)" suffix —
   * the component neither knows nor asks what makes an entry exclusive.
   */
  addedExclusiveIds: ReadonlySet<string>;
  draft: WhereToRunDraft;
  error: HostFailure | null;
  isDiscovering: boolean;
  isPending: boolean;
  onDiscover: () => void;
  onSelect: (harnessId: string) => void;
}) {
  const canDiscover = providerConfigComplete(draft) && !isPending;
  const harnesses = draft.remoteHarnesses;
  const options = remoteHarnessOptions(harnesses, addedExclusiveIds);

  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium" htmlFor="agent-remote-harness">
        Harness on the host
      </label>
      {harnesses === null ? (
        <p className="text-sm text-muted-foreground">
          Agents run the harness installed on the host, not on this computer.
        </p>
      ) : options.length === 0 ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          No supported harness is installed on that host. Install one (for
          example <span className="font-mono">goose</span>) and check again.
        </p>
      ) : (
        <PersonaDropdownField
          disabled={isPending}
          id="agent-remote-harness"
          onValueChange={onSelect}
          options={options}
          placeholder="Select a harness"
          value={draft.remoteHarnessId ?? ""}
        />
      )}
      {error ? (
        <div className="space-y-2">
          <p className="text-sm text-warning">{error.message}</p>
          <RecoveryAction recovery={error.recovery} />
        </div>
      ) : null}
      {/* Always available, in every state: a failed connection, an empty
          catalog and a just-installed harness all need a second attempt, and
          hiding the button after the first one strands the user in the create
          dialog with no path forward. */}
      <Button
        disabled={!canDiscover || isDiscovering}
        onClick={onDiscover}
        size="sm"
        type="button"
        variant="outline"
      >
        {isDiscovering ? (
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
        ) : null}
        {isDiscovering
          ? "Checking host…"
          : harnesses === null
            ? "Check the host"
            : "Check again"}
      </Button>
    </div>
  );
}

/**
 * The one actionable step a failed host call can offer, or nothing.
 *
 * Deliberately offers only "open the page". Nothing here schedules a retry: the
 * desktop cannot observe a browser it does not own, so waiting for the user to
 * finish authenticating would be guessing at a delay. "Check the host again" is
 * already the retry, and it is the same button every other host failure offers.
 *
 * The URL is not re-checked here — `ProviderRecovery::from_response` validated
 * it against the Tailscale login prefix and token charset on the way into the
 * desktop, before it was ever a value this process held. Adding a second check
 * here would imply the first one is optional.
 */
function RecoveryAction({ recovery }: { recovery: ProviderRecovery | null }) {
  if (!recovery) return null;
  return (
    <Button
      onClick={() => void openUrl(recovery.url)}
      size="sm"
      type="button"
      variant="outline"
    >
      <ExternalLink className="mr-2 h-4 w-4" />
      Authenticate in browser
    </Button>
  );
}
