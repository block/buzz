import type * as React from "react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { PinnedHarness } from "@/features/agents/lib/pinnedHarness";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";

/**
 * The Edit Agent dialog's harness controls, and the model control that depends
 * on them.
 *
 * Two records, two different questions. A LOCAL agent runs on this computer, so
 * this computer's catalog can offer it every harness it has — the dropdown is
 * unchanged, down to its "Provider" label and "Detected at" line.
 *
 * A provider-backed agent's harness lives on the HOST. The local catalog has
 * never seen it, so every answer it gives is either a miss (falling through to
 * the app's default runtime, which is how a Hermes agent came to describe
 * itself as "Buzz Agent") or a name collision. `pinnedHarness` is the record's
 * own pin, and it is the only thing here that knows anything: it is shown
 * read-only, because the pin is captured at create time and nothing in this
 * dialog can reach the host to change it.
 */
export function EditAgentHarnessFields({
  agentCommand,
  disabled,
  locationLabel,
  onAgentCommandChange,
  onRuntimeChange,
  pinnedHarness,
  runtimeOptions,
  runtimeValue,
  selectedRuntime,
  showCommandInput,
}: {
  agentCommand: string;
  disabled: boolean;
  /** `"on ssh"` — where a provider-backed agent runs. See `agentLocationLabel`. */
  locationLabel: string | null;
  onAgentCommandChange: (value: string) => void;
  onRuntimeChange: (value: string) => void;
  /** The record's harness pin, or `null` when it runs on this computer. */
  pinnedHarness: PinnedHarness | null;
  runtimeOptions: PersonaDropdownOption[];
  runtimeValue: string;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  showCommandInput: boolean;
}) {
  if (pinnedHarness) {
    return (
      <div className="space-y-1.5">
        <span className="block text-sm font-medium text-foreground">
          Agent harness
        </span>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
          data-testid="edit-agent-pinned-harness"
        >
          <span className="truncate text-sm leading-6 text-foreground">
            {pinnedHarness.label}
          </span>
        </div>
        <p className="text-xs text-muted-foreground">
          Runs <span className="font-medium">{pinnedHarness.command}</span>
          {locationLabel ? ` ${locationLabel}` : null}. Create a new agent to
          run a different harness there.
        </p>
      </div>
    );
  }

  return (
    <>
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-runtime"
        >
          Provider
        </label>
        <PersonaDropdownField
          disabled={disabled}
          id="edit-agent-runtime"
          onValueChange={onRuntimeChange}
          options={runtimeOptions}
          placeholder="Choose a provider"
          value={runtimeValue}
        />
        {selectedRuntime ? (
          <p className="text-xs text-muted-foreground">
            Detected at{" "}
            <span className="font-medium">
              {selectedRuntime.binaryPath ??
                selectedRuntime.command ??
                selectedRuntime.id}
            </span>
          </p>
        ) : null}
      </div>
      {showCommandInput ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-command"
          >
            Agent command
          </label>
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled}
              id="edit-agent-command"
              onChange={(event) => onAgentCommandChange(event.target.value)}
              placeholder="Full path or shell command"
              value={agentCommand}
            />
          </div>
        </div>
      ) : null}
    </>
  );
}

/**
 * The Model control for a provider-backed record.
 *
 * The local Model control is a dropdown over a catalog this computer probed by
 * running a harness binary here. A provider-backed agent's harness is on the
 * host, so that probe describes the wrong machine — and the dropdown left with
 * nothing to offer is the empty "Choose a model" the fleet actually shipped.
 *
 * What this record does know is its own model, so that is what is shown, in the
 * same custom-model input the local path already falls back to when a model is
 * outside the discovered catalog. It stays editable because a saved model does
 * reach the host: the deploy payload re-resolves it on the next start.
 */
export function EditAgentPinnedModelField({
  disabled,
  harnessLabel,
  model,
  modelBlockedMessage,
  onModelChange,
  required,
}: {
  disabled: boolean;
  /** The pinned harness's human label, for the hint line. */
  harnessLabel: string;
  model: string;
  /**
   * Why this model change cannot be saved, when it cannot. Present means Save
   * is blocked, so it replaces the hint line rather than sitting beneath it —
   * this sentence explains a dead button and must not read as an aside. See
   * `resolveInstanceModelDefinitionWrite`.
   */
  modelBlockedMessage?: string;
  onModelChange: (value: string) => void;
  required: boolean;
}) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-model"
      >
        Model
        {required ? (
          <span className="ml-1 text-destructive" aria-hidden="true">
            *
          </span>
        ) : (
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        )}
      </label>
      <div
        className={cn(
          "flex min-h-11 items-center px-3",
          PERSONA_FIELD_SHELL_CLASS,
        )}
      >
        <Input
          autoCorrect="off"
          className={cn("h-8 px-0 py-0 leading-6", PERSONA_FIELD_CONTROL_CLASS)}
          data-testid="edit-agent-pinned-model"
          disabled={disabled}
          id="edit-agent-model"
          onChange={(event) => onModelChange(event.target.value)}
          placeholder="Harness default"
          value={model}
        />
      </div>
      {modelBlockedMessage ? (
        <p className="text-xs text-destructive" role="alert">
          {modelBlockedMessage}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          Models come from the host, which this computer cannot list. Enter an
          id {harnessLabel} supports there, or leave it empty for its default.
          Saved changes take effect on the next start.
        </p>
      )}
    </div>
  );
}

/**
 * The Model control for a LOCAL record: a dropdown over the catalog this
 * computer probed, with the custom-model input the dropdown falls back to and
 * the discovery status line.
 *
 * Extracted from `AgentInstanceEditDialog` so it sits beside the pinned
 * counterpart above — the two are the same field answering the two different
 * questions this file already documents, and keeping them apart made the split
 * read as an accident of file size rather than a rule.
 */
export function EditAgentLocalModelField({
  customModelVisible,
  disabled,
  discoveryLoading,
  model,
  modelBlocked,
  onModelChange,
  onModelSelect,
  options,
  required,
  selectValue,
  statusMessage,
}: {
  /** The dropdown resolved to "Custom model…", so the free-text input shows. */
  customModelVisible: boolean;
  disabled: boolean;
  discoveryLoading: boolean;
  model: string;
  /** The status line explains a dead Save button, so it cannot render as a hint. */
  modelBlocked: boolean;
  onModelChange: (value: string) => void;
  onModelSelect: (value: string) => void;
  options: PersonaDropdownOption[];
  required: boolean;
  selectValue: string;
  statusMessage: string | null | undefined;
}) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-model"
      >
        Model
        {required ? (
          <span className="ml-1 text-destructive" aria-hidden="true">
            *
          </span>
        ) : (
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        )}
      </label>
      <PersonaDropdownField
        disabled={disabled || discoveryLoading}
        id="edit-agent-model"
        onValueChange={onModelSelect}
        options={options}
        placeholder="Default model"
        value={selectValue}
      />
      {customModelVisible ? (
        <div
          className={cn(
            "mt-2 flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            aria-label="Custom model ID"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="edit-agent-custom-model"
            onChange={(event) => onModelChange(event.target.value)}
            placeholder="Custom model ID"
            value={model}
          />
        </div>
      ) : null}
      {statusMessage ? (
        <p
          className={cn(
            "text-xs",
            modelBlocked ? "text-warning" : "text-muted-foreground",
          )}
        >
          {statusMessage}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The instance dialog's Model field: the pinned control for a provider-backed
 * record, the catalog dropdown for a local one.
 *
 * The choice lives here rather than at the call site because this file already
 * owns both halves and the reason they differ — the host's catalog is
 * unreachable from this computer, so a pinned record names its own model
 * instead of being offered this machine's. Splitting the branch from the
 * branches would put half that explanation in a dialog that otherwise never
 * needs to know a remote catalog exists.
 */
export function EditAgentModelField({
  modelBlockedMessage,
  pinnedHarness,
  ...local
}: React.ComponentProps<typeof EditAgentLocalModelField> & {
  /**
   * Why a model change cannot be saved. Only a pinned record can be blocked —
   * the block comes from the definition write the pinned path performs (see
   * `useInstanceModelDefinitionWrite`), and the local path has no such write.
   */
  modelBlockedMessage?: string;
  /** The record's harness pin, or `null` when it runs on this computer. */
  pinnedHarness: PinnedHarness | null;
}) {
  if (pinnedHarness) {
    return (
      <EditAgentPinnedModelField
        disabled={local.disabled}
        harnessLabel={pinnedHarness.label}
        model={local.model}
        modelBlockedMessage={modelBlockedMessage}
        onModelChange={local.onModelChange}
        required={local.required}
      />
    );
  }
  return <EditAgentLocalModelField {...local} />;
}
