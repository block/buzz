import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  formatRuntimeOptionLabel,
  NO_RUNTIME_DROPDOWN_VALUE,
  type PersonaDropdownOption,
  sortPersonaRuntimes,
} from "./agentConfigOptions";

/**
 * How much the LOCAL runtime catalog is allowed to gate a definition create.
 *
 * A local create must name a runtime installed on this computer. A remote
 * create must not: its harness comes from the host's catalog via the
 * "Where to run" section, and the local catalog describes a different machine
 * entirely — requiring a local install would make every remote-only harness
 * unsubmittable (no Goose agent on a server without Goose on the laptop).
 *
 * `createSubmitBlocked` still gates the remote case; it is false only once a
 * remote harness has actually been picked.
 */
export type CreateRuntimeGateInput = {
  isCreateMode: boolean;
  /** "Where to run" targets a backend provider. */
  runsRemotely: boolean;
  /** The definition's runtime id, as typed/selected. */
  runtime: string;
  selectedRuntime: AcpRuntimeCatalogEntry | null | undefined;
  /** True when the app could resolve any default runtime locally. */
  hasLocalDefaultRuntime: boolean;
};

/**
 * The harness id every credential question must be asked of.
 *
 * A local create asks the local catalog. A remote one asks the HOST's pin: the
 * deploy writes this agent's env on the host keyed off the remote command
 * (`deploy.rs::metadata_env`), while the dialog's `runtime` still holds
 * whatever the local seeding effects resolved from this computer's catalog.
 * Asking the local id makes the two machines disagree about which env keys
 * matter — a remote Goose on a Claude-defaulted laptop is told it needs no
 * credentials at all, because `runtimeSupportsLlmProviderSelection` answers
 * false for `claude` and the requirement list comes back empty.
 *
 * The id spaces are identical by construction: the SSH provider's discovery
 * emits exactly the `goose` and `buzz-agent` keys the local catalog and
 * `metadata_env` both use. `""` for an unpinned remote harness is the honest
 * answer — there is no harness to demand credentials for yet, and the create
 * is already blocked until one is picked.
 */
export function createGateHarnessId({
  runsRemotely,
  runtime,
  remoteHarnessId,
}: {
  runsRemotely: boolean;
  runtime: string;
  remoteHarnessId: string | null;
}): string {
  return runsRemotely ? (remoteHarnessId ?? "") : runtime;
}

/**
 * Whether the dialog may seed its harness field from this computer's default.
 *
 * Never when the agent runs somewhere else. `runtime` is the definition's
 * harness preference, and a provider-backed agent takes its harness from the
 * HOST's catalog — so seeding the local default stamps a harness of the wrong
 * machine (`buzz-agent` on most installs) onto a record that runs somewhere
 * else, and every surface reading the record back reports it as the harness the
 * agent runs on. The remote pin travels via `BackendIntent.harness` instead.
 *
 * `targetsRemoteHost` covers both shapes of that one fact: "Where to run"
 * pointing at a provider during a create, and an edit whose record is already
 * provider-backed.
 */
export function createRuntimeSeedAllowed(targetsRemoteHost: boolean): boolean {
  return !targetsRemoteHost;
}

/**
 * What the harness auto-seed should do on this render.
 *
 * `shed` exists because "Where to run" lives inside the dialog and starts
 * local: the seed has usually already been applied by the time the user picks a
 * provider, so refusing to seed is not enough on its own — the stamped local
 * default has to be taken back off, or the remote create submits it anyway.
 * Only an auto-seeded value is ever shed; an explicit pick belongs to the user.
 */
export type CreateRuntimeSeedAction =
  | { type: "seed"; runtimeId: string }
  | { type: "shed" }
  | { type: "none" };

export function createRuntimeSeedAction({
  defaultRuntimeId,
  definitionRuntime,
  editsProviderRecord = false,
  hasInitialValues,
  hasSeededForOpen,
  isAutoSeeded,
  open,
  runsRemotely,
  runtime,
  runtimesLoading,
}: {
  /** The local default, or null when nothing is installed. */
  defaultRuntimeId: string | null;
  /** The definition's own runtime preference, which the seed never overrides. */
  definitionRuntime: string | null | undefined;
  /**
   * This EDIT is opened for a provider-backed record.
   *
   * `runsRemotely` answers the same question for a create, from a control that
   * only exists there — so it is false in edit mode, and the seed happily
   * stamped this computer's default onto a definition whose record runs
   * somewhere else. The record's blank runtime is not an absence to be filled:
   * `to_definition_view` drops the harness on purpose, because the real one is
   * the host's. Belt and braces behind the routing fixes — a surface that still
   * reaches this dialog for a remote record must not invent a harness for it.
   */
  editsProviderRecord?: boolean;
  hasInitialValues: boolean;
  hasSeededForOpen: boolean;
  isAutoSeeded: boolean;
  open: boolean;
  runsRemotely: boolean;
  runtime: string;
  runtimesLoading: boolean;
}): CreateRuntimeSeedAction {
  if (!createRuntimeSeedAllowed(runsRemotely || editsProviderRecord)) {
    return isAutoSeeded ? { type: "shed" } : { type: "none" };
  }
  if (
    !open ||
    !hasInitialValues ||
    definitionRuntime?.trim() ||
    runtimesLoading ||
    runtime.trim().length > 0 ||
    defaultRuntimeId === null ||
    hasSeededForOpen
  ) {
    return { type: "none" };
  }
  return { type: "seed", runtimeId: defaultRuntimeId };
}

/** Whether the picked runtime clears the local-availability requirement. */
export function createRuntimeIsAvailable({
  runsRemotely,
  runtime,
  selectedRuntime,
}: Pick<
  CreateRuntimeGateInput,
  "runsRemotely" | "runtime" | "selectedRuntime"
>): boolean {
  if (runsRemotely) return true;
  if (runtime.trim().length === 0) return true;
  return selectedRuntime?.availability === "available";
}

/** Whether the runtime field satisfies the create-mode requirements. */
export function createRuntimeSelectionSatisfied(
  input: CreateRuntimeGateInput,
): boolean {
  if (!input.isCreateMode) return true;
  if (input.runsRemotely) return true;
  return input.runtime.trim().length > 0 && createRuntimeIsAvailable(input);
}

/**
 * Whether an unavailable runtime option should be unselectable. Remote creates
 * never disable an option: availability here describes the wrong machine.
 */
export function createRuntimeOptionDisabled(
  candidate: AcpRuntimeCatalogEntry,
  input: Pick<
    CreateRuntimeGateInput,
    "isCreateMode" | "runsRemotely" | "hasLocalDefaultRuntime"
  >,
): boolean {
  return (
    input.isCreateMode &&
    !input.runsRemotely &&
    input.hasLocalDefaultRuntime &&
    candidate.availability !== "available"
  );
}

/**
 * Label for the "no explicit runtime" state: the placeholder in create mode,
 * and an actual selectable option when editing (where blank is legitimate).
 */
export function runtimeDropdownPlaceholder({
  isCreateMode,
  runtimesLoading,
}: {
  isCreateMode: boolean;
  runtimesLoading: boolean;
}): string {
  if (runtimesLoading) return "Loading harnesses...";
  return isCreateMode ? "Choose a harness" : "No preference (use app default)";
}

/**
 * The harness dropdown for the definition dialog: catalog order, the gate's
 * disabled flags, and a trailing entry for a runtime the catalog no longer
 * knows so an existing definition never silently loses its own value.
 */
export function runtimeDropdownOptions({
  gate,
  defaultRuntimeId,
  runtimes,
  runtimesLoading,
}: {
  gate: CreateRuntimeGateInput;
  defaultRuntimeId: string | null;
  runtimes: readonly AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
}): PersonaDropdownOption[] {
  const options: PersonaDropdownOption[] = [
    ...(gate.isCreateMode
      ? []
      : [
          {
            label: runtimeDropdownPlaceholder({
              isCreateMode: gate.isCreateMode,
              runtimesLoading,
            }),
            value: NO_RUNTIME_DROPDOWN_VALUE,
          },
        ]),
    ...sortPersonaRuntimes(runtimes).map((candidate) => ({
      disabled: createRuntimeOptionDisabled(candidate, gate),
      label: `${formatRuntimeOptionLabel(candidate)}${
        gate.isCreateMode && candidate.id === defaultRuntimeId
          ? " (default)"
          : ""
      }`,
      value: candidate.id,
    })),
  ];
  const current = gate.runtime.trim();
  if (
    current.length > 0 &&
    !options.some((option) => option.value === gate.runtime)
  ) {
    options.push({ label: `${current} (current)`, value: current });
  }
  return options;
}
