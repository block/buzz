import { backendProviderLabel } from "../lib/backendProviderLabel";
import type { BackendIntent } from "../lib/instanceInputForDefinition";
import { providerRecoveryOf, type ProviderRecovery } from "@/shared/api/tauri";
import type {
  AgentModelsResponse,
  BackendProviderCandidate,
  BackendProviderProbeResult,
  RemoteHarness,
} from "@/shared/api/types";
import type { PersonaDropdownOption } from "./agentConfigOptions";
import { coerceConfigValues } from "./ProviderConfigFields";
import type { ModelDiscoveryView } from "./useRemoteAwareModelDiscovery";
import { getDiscoveredPersonaModelOptions } from "./usePersonaModelDiscovery";

/**
 * The model catalog of the picked remote harness, read from the HOST by
 * `probe_provider_models`.
 *
 * A provider-backed agent runs its harness on the host, so its models are the
 * host's models. The local discovery path would answer with this computer's
 * catalog — a different machine, and for a remote-only harness usually an
 * empty or failed one — which is why a remote create reads this instead.
 */
export type RemoteModelProbe =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "loaded"; models: AgentModelsResponse }
  | { status: "failed"; error: string };

/**
 * A host failure the user can act on, as the section renders it.
 *
 * The message always stands alone — it names the problem, and for the
 * Tailscale case it carries the URL as text — so `recovery` only ever adds a
 * button. A failure without one is an ordinary failure with a `null` here.
 */
export type HostFailure = {
  message: string;
  recovery: ProviderRecovery | null;
};

/**
 * Read a rejected host call into the shape the section renders.
 *
 * Every `catch` in this flow goes through here so the recovery is picked up in
 * one place rather than at each call site — the failure paths differ in which
 * state they write, not in how they read an error.
 */
export function hostFailureOf(error: unknown): HostFailure {
  return {
    message: error instanceof Error ? error.message : String(error),
    recovery: providerRecoveryOf(error),
  };
}

/** Dropdown value of the "runs on this computer" choice. */
export const LOCAL_RUN_TARGET_VALUE = "local";

/** Draft state of the optional remote-backend selector. */
export type WhereToRunDraft = {
  /** `LOCAL_RUN_TARGET_VALUE`, or the id of a discovered backend provider. */
  runOn: string;
  providerConfig: Record<string, string>;
  probedProvider: BackendProviderProbeResult | null;
  /**
   * The harness catalog of the REMOTE host, once `discover_provider_harnesses`
   * has run against the entered config. `null` means "not discovered yet".
   */
  remoteHarnesses: readonly RemoteHarness[] | null;
  /** Id of the picked entry of `remoteHarnesses`. */
  remoteHarnessId: string | null;
  /** Models of the picked entry, probed on the host. */
  remoteModelProbe: RemoteModelProbe;
};

export const emptyWhereToRunDraft: WhereToRunDraft = {
  runOn: LOCAL_RUN_TARGET_VALUE,
  providerConfig: {},
  probedProvider: null,
  remoteHarnesses: null,
  remoteHarnessId: null,
  remoteModelProbe: { status: "idle" },
};

/**
 * The run-target choices: this computer, then every discovered backend
 * provider.
 *
 * A provider's own `info.name` ("SSH") is friendlier than its binary-derived id
 * ("ssh"), but `info` is a subprocess round-trip and this list is rendered
 * before the user has asked for anything, so only providers the user has
 * actually selected have ever been probed. `probedNames` carries the names
 * already paid for — see `rememberProbedProviderName` — and the id stands in
 * for the rest, rather than spawning every discovered provider on dialog open
 * to decorate a label. The name-or-id choice itself is
 * `backendProviderLabel`'s, so the create dialog and the agent cards cannot
 * drift into two naming schemes for one machine.
 *
 * The cache is what keeps the list stable. Reading the name off the CURRENT
 * selection alone would rename a provider the moment it is picked and rename
 * it back when it is not, so the same machine would appear under two naming
 * schemes depending on where the cursor is, and a label would mutate under the
 * user when a probe resolved.
 */
export function runTargetOptions(
  providers: readonly BackendProviderCandidate[],
  probedNames: Readonly<Record<string, string>>,
): PersonaDropdownOption[] {
  return [
    { label: "This computer", value: LOCAL_RUN_TARGET_VALUE },
    ...providers.map((provider) => ({
      label: backendProviderLabel(provider.id, probedNames[provider.id]),
      value: provider.id,
    })),
  ];
}

/**
 * Fold a completed probe into the cache of friendly provider names.
 *
 * Returns the SAME object when there is nothing to add, so the caller can use
 * it as a state updater without re-rendering on every probe of a provider
 * already named. A blank or missing name is not cached: the id is a better
 * label than an empty one.
 */
export function rememberProbedProviderName(
  probedNames: Readonly<Record<string, string>>,
  providerId: string,
  probed: BackendProviderProbeResult | null,
): Readonly<Record<string, string>> {
  const name = probed?.name?.trim();
  if (!name || providerId === LOCAL_RUN_TARGET_VALUE) return probedNames;
  if (probedNames[providerId] === name) return probedNames;
  return { ...probedNames, [providerId]: name };
}

export function providerConfigComplete(draft: WhereToRunDraft): boolean {
  if (draft.runOn === LOCAL_RUN_TARGET_VALUE) return true;
  if (!draft.probedProvider) return false;
  const schema = draft.probedProvider.config_schema as
    | Record<string, unknown>
    | undefined;
  const required: string[] = (schema?.required as string[] | undefined) ?? [];
  return required.every(
    (key) => (draft.providerConfig[key] ?? "").trim().length > 0,
  );
}

/**
 * The dropdown rows for the host's harness catalog.
 *
 * Unavailable entries are omitted (the pin must name a binary the host
 * reported, see `selectedRemoteHarness`). Entries in `addedExclusiveIds` stay
 * VISIBLE but disabled: an exclusive entry is a persistent identity on the host
 * that an existing agent already drives, and hiding it would read as "that
 * profile is gone" rather than "it is already yours". The "(added)" suffix is
 * the same parenthetical annotation `formatRuntimeOptionLabel` uses for
 * "(not installed)" — the picker has one label vocabulary, not two.
 *
 * Pure, and agnostic about what makes an entry exclusive or added: the caller
 * computes that set (`addedExclusiveHarnessIds`) and this only renders it.
 */
export function remoteHarnessOptions(
  harnesses: readonly RemoteHarness[] | null,
  addedExclusiveIds: ReadonlySet<string>,
): PersonaDropdownOption[] {
  return (harnesses ?? [])
    .filter((harness) => harness.available)
    .map((harness) => {
      const added = addedExclusiveIds.has(harness.id);
      const version = harness.version ? ` (${harness.version})` : "";
      return {
        label: `${harness.label}${version}${added ? " (added)" : ""}`,
        value: harness.id,
        ...(added ? { disabled: true } : {}),
      };
    });
}

/**
 * What the harness picker should select after a catalog read.
 *
 * Keeps an existing pick when the re-check still offers it, otherwise falls to
 * the first entry that can be picked at all — so the common case needs no extra
 * interaction. An added-exclusive entry is never either of those: auto-picking
 * one would silently arm a create the picker itself refuses, and submitting it
 * would put a second agent on an identity that already has one.
 *
 * Returns the entry rather than its id because the caller immediately probes
 * the host for its models.
 */
export function autoPickRemoteHarness(
  harnesses: readonly RemoteHarness[],
  addedExclusiveIds: ReadonlySet<string>,
  previousId: string | null,
): RemoteHarness | null {
  const selectable = harnesses.filter(
    (harness) => harness.available && !addedExclusiveIds.has(harness.id),
  );
  return (
    selectable.find((harness) => harness.id === previousId) ??
    selectable[0] ??
    null
  );
}

/**
 * The picked remote harness, or null when none is selected/available.
 *
 * Only an `available` catalog entry can be the pick. An unavailable entry names
 * a harness the host reported as not installed, so pinning it would ship a
 * command that fails at deploy time — after the create has already succeeded.
 * The picker never offers those entries, but a re-check can turn a previously
 * available id unavailable while it is still selected, so the narrowing lives
 * here (the single owner of "what is pinned") rather than in the component.
 */
export function selectedRemoteHarness(
  draft: WhereToRunDraft,
): RemoteHarness | null {
  if (draft.runOn === LOCAL_RUN_TARGET_VALUE || !draft.remoteHarnessId)
    return null;
  return (
    draft.remoteHarnesses?.find(
      (harness) => harness.available && harness.id === draft.remoteHarnessId,
    ) ?? null
  );
}

/**
 * How the dialog's summary names the harness for a provider-backed create.
 *
 * `null` means "the local path owns this label", exactly as
 * `remoteModelDiscoveryView` does for the Model control — a local create, or a
 * remote one with nothing picked yet, still reads from the local catalog.
 */
export function remoteHarnessSummaryLabel(
  draft: WhereToRunDraft,
): string | null {
  const harness = selectedRemoteHarness(draft);
  if (!harness) return null;
  return harness.version
    ? `${harness.label} (${harness.version})`
    : harness.label;
}

/**
 * What the create dialog's Model control renders for a provider-backed create.
 *
 * Deliberately the same shape `usePersonaModelDiscovery` returns, so the
 * dialog swaps one for the other rather than growing a parallel remote
 * rendering path. `harnessId` is the reset key: changing the harness resets
 * the dependent model exactly as changing the local one does.
 */
export type RemoteModelDiscoveryView = ModelDiscoveryView & {
  harnessId: string;
};

/**
 * Project the host's model probe into the dialog's Model control.
 *
 * `null` means "the local path owns this control": either the agent runs
 * locally, or no remote harness has been picked yet so there is nothing to
 * have probed.
 *
 * The status copy is remote-specific on purpose. The local failure copy
 * ("using built-in model options") is a lie here — there is no built-in
 * catalog for someone else's machine, and the actionable step is on the host,
 * not in this dialog.
 */
export function remoteModelDiscoveryView(
  draft: WhereToRunDraft,
): RemoteModelDiscoveryView | null {
  const harness = selectedRemoteHarness(draft);
  if (!harness) return null;
  const probe = draft.remoteModelProbe;
  if (probe.status === "idle") return null;

  const base = {
    harnessId: harness.id,
    modelDiscoveryLoading: probe.status === "loading",
  };
  if (probe.status === "loading") {
    return {
      ...base,
      discoveredModelOptions: null,
      modelDiscoveryStatus: null,
    };
  }
  if (probe.status === "failed") {
    return {
      ...base,
      discoveredModelOptions: null,
      modelDiscoveryStatus: {
        // Name the retry explicitly. The probe reads the definition's env at
        // call time, so typing a missing API key afterwards does not re-probe
        // by itself — without this the auth-error case looks like a dead end.
        message: `Could not load models from the host: ${probe.error}. Fix it on the host (or in this agent's credentials), then check the host again.`,
        tone: "warning",
      },
    };
  }

  // Provider is fixed as "" rather than the definition's: that argument only
  // decides whether a "Default model" row is offered, and for a remote harness
  // the host's own default is always a legitimate choice.
  const options = getDiscoveredPersonaModelOptions(probe.models, "");
  return {
    ...base,
    discoveredModelOptions: options,
    modelDiscoveryStatus:
      options === null
        ? {
            message: `${
              probe.models.agentName.trim() || "That harness"
            } reported no models on the host. Check that it is installed and signed in there, then check the host again.`,
            tone: "warning",
          }
        : null,
  };
}

/**
 * A provider create must carry a harness from the remote catalog: it is the
 * only channel by which the harness choice reaches the host, and without it the
 * record would fall back to the locally-resolved default (`buzz-agent`) and the
 * host would silently provision a harness the user never chose. So the submit
 * button stays blocked until one is picked, rather than letting the create fail
 * later inside `buildInstanceInputForDefinition`.
 */
export function canSubmitWhereToRun(draft: WhereToRunDraft): boolean {
  if (!providerConfigComplete(draft)) return false;
  if (draft.runOn === LOCAL_RUN_TARGET_VALUE) return true;
  return selectedRemoteHarness(draft) !== null;
}

export function resolveBackendIntent(
  draft: WhereToRunDraft,
): BackendIntent | null {
  if (draft.runOn === LOCAL_RUN_TARGET_VALUE) return null;
  const harness = selectedRemoteHarness(draft);
  return {
    type: "provider",
    id: draft.runOn,
    config: coerceConfigValues(
      draft.providerConfig,
      draft.probedProvider?.config_schema,
    ),
    ...(harness
      ? {
          harness: {
            id: harness.id,
            command: harness.command,
            args: harness.args,
            env: harness.env,
          },
        }
      : {}),
  };
}
