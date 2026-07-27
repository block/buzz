import * as React from "react";
import { ExternalLink, Plus, Terminal, Trash2, X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpRuntimesQuery,
  useDeleteCustomHarnessMutation,
  useManagedAgentPrereqsQuery,
  useManagedAgentsQuery,
  usePersonasQuery,
  useSaveCustomHarnessMutation,
} from "@/features/agents/hooks";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

import {
  getRuntimeDisplayLabel,
  RuntimeIcon,
} from "../../onboarding/ui/RuntimeIcon";
import {
  commaArgError,
  type CustomFormValues,
  definitionFromFormValues,
  formValuesFromCatalogEntry,
  idFromLabel,
} from "./harnessFormLogic";
import {
  customEntries as getCustomEntries,
  deleteConfirmState,
  sortedPresetEntries,
} from "./harnessGalleryLogic";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

// ── Preset card ───────────────────────────────────────────────────────────────
//
// Preset entries come from the backend catalog (source === "preset"), so we no
// longer maintain a duplicate HARNESS_PRESETS array here. The backend
// PRESET_HARNESSES static drives availability detection and canonical data.

function PresetCard({ entry }: { entry: AcpRuntimeCatalogEntry }) {
  const isDetected = entry.availability === "available";

  return (
    <div
      className={cn(
        "relative flex flex-col gap-3 rounded-2xl border px-4 py-4 text-sm transition-colors",
        isDetected
          ? "border-emerald-500/20 bg-emerald-500/5"
          : "border-border/60 bg-muted/20",
      )}
      data-testid={`harness-preset-${entry.id}`}
    >
      <div className="flex items-center gap-3">
        <RuntimeIcon className="h-8 w-8" runtime={entry} />
        <div className="min-w-0 flex-1">
          <p className="font-medium leading-none">
            {getRuntimeDisplayLabel(entry)}
          </p>
          {entry.command ? (
            <p className="mt-1 font-mono text-xs text-muted-foreground">
              {entry.command}
            </p>
          ) : null}
        </div>
        {isDetected ? (
          <span className="inline-flex shrink-0 items-center rounded-md bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
            Detected
          </span>
        ) : null}
      </div>

      {/* Docs link for non-detected presets */}
      {!isDetected && entry.installInstructionsUrl ? (
        <div className="flex items-center gap-2">
          <button
            className="inline-flex items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
            onClick={() => void openUrl(entry.installInstructionsUrl)}
            type="button"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            Install
          </button>
        </div>
      ) : null}
    </div>
  );
}

// ── Custom harness form ───────────────────────────────────────────────────────

const EMPTY_FORM: CustomFormValues = {
  id: "",
  label: "",
  command: "",
  args: [],
  env: [],
  installInstructionsUrl: "",
  installHint: "",
};

function CommandAvailabilityBadge({ command }: { command: string }) {
  const trimmed = command.trim();
  const prereqs = useManagedAgentPrereqsQuery(trimmed, "", {
    enabled: trimmed.length > 0,
  });

  if (!trimmed || prereqs.isLoading) return null;

  const available = prereqs.data?.acp.available;
  if (available === undefined) return null;

  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md px-2 py-0.5 text-xs font-medium",
        available
          ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
          : "bg-amber-500/15 text-amber-600 dark:text-amber-400",
      )}
    >
      {available ? "Found on PATH" : "Not found on PATH"}
    </span>
  );
}

function ArgsEditor({
  args,
  onChange,
}: {
  args: string[];
  onChange: (next: string[]) => void;
}) {
  function set(index: number, value: string) {
    const next = [...args];
    next[index] = value;
    onChange(next);
  }

  function add() {
    onChange([...args, ""]);
  }

  function remove(index: number) {
    onChange(args.filter((_, i) => i !== index));
  }

  return (
    <div className="space-y-1.5">
      {args.map((arg, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: positional arg list
        <div className="flex gap-1.5" key={i}>
          <Input
            className="h-8 flex-1 font-mono text-sm"
            onChange={(e) => set(i, e.target.value)}
            placeholder={`arg ${i + 1}`}
            value={arg}
          />
          <button
            aria-label="Remove argument"
            className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
            onClick={() => remove(i)}
            type="button"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <Button
        className="h-7 gap-1.5 px-3 text-xs"
        onClick={add}
        size="sm"
        type="button"
        variant="ghost"
      >
        <Plus className="h-3.5 w-3.5" />
        Add argument
      </Button>
    </div>
  );
}

function EnvEditor({
  env,
  onChange,
}: {
  env: Array<{ key: string; value: string }>;
  onChange: (next: Array<{ key: string; value: string }>) => void;
}) {
  function set(index: number, field: "key" | "value", value: string) {
    const next = env.map((e, i) =>
      i === index ? { ...e, [field]: value } : e,
    );
    onChange(next);
  }

  function add() {
    onChange([...env, { key: "", value: "" }]);
  }

  function remove(index: number) {
    onChange(env.filter((_, i) => i !== index));
  }

  return (
    <div className="space-y-1.5">
      {env.map((pair, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: positional env list
        <div className="flex gap-1.5" key={i}>
          <Input
            className="h-8 w-1/3 font-mono text-sm"
            onChange={(e) => set(i, "key", e.target.value)}
            placeholder="KEY"
            value={pair.key}
          />
          <Input
            className="h-8 flex-1 font-mono text-sm"
            onChange={(e) => set(i, "value", e.target.value)}
            placeholder="value"
            value={pair.value}
          />
          <button
            aria-label="Remove env var"
            className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
            onClick={() => remove(i)}
            type="button"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <Button
        className="h-7 gap-1.5 px-3 text-xs"
        onClick={add}
        size="sm"
        type="button"
        variant="ghost"
      >
        <Plus className="h-3.5 w-3.5" />
        Add env var
      </Button>
    </div>
  );
}

function CustomHarnessForm({
  initial,
  originalId,
  onCancel,
  onSaved,
}: {
  initial?: Partial<CustomFormValues>;
  /** Id of the harness being edited, if this is an edit (not new). Used to
   * delete the old file when the id changes. */
  originalId?: string;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const [form, setForm] = React.useState<CustomFormValues>({
    ...EMPTY_FORM,
    ...initial,
  });
  const [error, setError] = React.useState<string | null>(null);
  const save = useSaveCustomHarnessMutation();

  function field(
    key: keyof Pick<
      CustomFormValues,
      "id" | "label" | "command" | "installInstructionsUrl" | "installHint"
    >,
  ) {
    return (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value;
      setForm((prev) => {
        const next = { ...prev, [key]: value };
        // Auto-derive id from label when id is empty or was auto-derived.
        if (
          key === "label" &&
          (!prev.id || prev.id === idFromLabel(prev.label))
        ) {
          next.id = idFromLabel(value);
        }
        return next;
      });
    };
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    // Mirror the backend comma-in-args rejection so the user gets an inline
    // error naming the offending argument before the round-trip.
    const commaError = commaArgError(form.args);
    if (commaError) {
      setError(commaError);
      return;
    }
    try {
      await save.mutateAsync({
        definition: definitionFromFormValues(form),
        originalId,
      });
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <form
      className="space-y-3 rounded-2xl border border-border/60 bg-muted/10 px-4 py-4"
      data-testid="custom-harness-form"
      onSubmit={(e) => void handleSubmit(e)}
    >
      <div className="flex items-center justify-between">
        <p className="text-sm font-medium">
          {originalId ? "Edit harness" : "Add custom harness"}
        </p>
        <button
          aria-label="Cancel"
          className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          onClick={onCancel}
          type="button"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-1">
          <p className="text-xs text-muted-foreground">Name</p>
          <Input
            className="h-8 text-sm"
            id="ch-label"
            onChange={field("label")}
            placeholder="My Runtime"
            required
            value={form.label}
          />
        </div>
        <div className="space-y-1">
          <p className="text-xs text-muted-foreground">
            ID <span className="text-muted-foreground/60">(auto-derived)</span>
          </p>
          <Input
            className="h-8 font-mono text-sm"
            id="ch-id"
            onChange={field("id")}
            placeholder="my-runtime"
            required
            value={form.id}
          />
        </div>
      </div>

      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <p className="text-xs text-muted-foreground">Command</p>
          <CommandAvailabilityBadge command={form.command} />
        </div>
        <Input
          className="h-8 font-mono text-sm"
          id="ch-command"
          onChange={field("command")}
          placeholder="my-agent-bin"
          required
          value={form.command}
        />
      </div>

      <div className="space-y-1">
        <p className="text-xs text-muted-foreground">Arguments</p>
        <ArgsEditor
          args={form.args}
          onChange={(args) => setForm((p) => ({ ...p, args }))}
        />
      </div>

      <div className="space-y-1">
        <p className="text-xs text-muted-foreground">
          Env vars{" "}
          <span className="text-muted-foreground/60">
            (override at spawn time; Buzz-managed vars always win)
          </span>
        </p>
        <EnvEditor
          env={form.env}
          onChange={(env) => setForm((p) => ({ ...p, env }))}
        />
      </div>

      <div className="space-y-1">
        <p className="text-xs text-muted-foreground">
          Docs URL <span className="text-muted-foreground/60">(optional)</span>
        </p>
        <Input
          className="h-8 text-sm"
          id="ch-docs-url"
          onChange={field("installInstructionsUrl")}
          placeholder="https://example.com/docs"
          value={form.installInstructionsUrl}
        />
      </div>

      <div className="space-y-1">
        <p className="text-xs text-muted-foreground">
          Install hint{" "}
          <span className="text-muted-foreground/60">(optional)</span>
        </p>
        <Input
          className="h-8 text-sm"
          id="ch-install-hint"
          onChange={field("installHint")}
          placeholder="npm install -g my-harness"
          value={form.installHint}
        />
      </div>

      {error ? (
        <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex justify-end gap-2 pt-1">
        <Button onClick={onCancel} size="sm" type="button" variant="outline">
          Cancel
        </Button>
        <Button disabled={save.isPending} size="sm" type="submit">
          {save.isPending ? <Spinner className="mr-2 h-3.5 w-3.5" /> : null}
          Save
        </Button>
      </div>
    </form>
  );
}

// ── Custom harness row ────────────────────────────────────────────────────────

function CustomHarnessRow({ entry }: { entry: AcpRuntimeCatalogEntry }) {
  const [editing, setEditing] = React.useState(false);
  const [confirmingDelete, setConfirmingDelete] = React.useState(false);
  const [deleteError, setDeleteError] = React.useState<string | null>(null);
  const del = useDeleteCustomHarnessMutation();
  // Blast-radius data for the delete confirmation — only fetched while the
  // confirmation is open, so the row list doesn't poll agents. Confirm stays
  // disabled until both queries settle (deleteConfirmState) so a quick click
  // can't beat the "N agents will stop launching" warning.
  const agentsQuery = useManagedAgentsQuery({ enabled: confirmingDelete });
  const personasQuery = usePersonasQuery({ enabled: confirmingDelete });
  const confirmState = deleteConfirmState(
    entry.id,
    entry.label,
    agentsQuery,
    personasQuery,
  );

  if (editing) {
    return (
      <CustomHarnessForm
        initial={formValuesFromCatalogEntry(entry)}
        originalId={entry.id}
        onCancel={() => setEditing(false)}
        onSaved={() => setEditing(false)}
      />
    );
  }

  return (
    <div
      className="flex flex-col gap-2 rounded-2xl border border-border/60 bg-muted/20 px-4 py-3 text-sm"
      data-testid={`custom-harness-row-${entry.id}`}
    >
      <div className="flex items-center gap-3">
        <Terminal className="h-6 w-6 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="font-medium leading-none">{entry.label}</p>
          <p className="mt-0.5 font-mono text-xs text-muted-foreground">
            {entry.command ?? entry.id}
            {(entry.defaultArgs ?? []).length > 0
              ? ` ${(entry.defaultArgs ?? []).join(" ")}`
              : ""}
          </p>
        </div>
        {entry.availability === "available" ? (
          <span className="inline-flex shrink-0 items-center rounded-md bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
            Detected
          </span>
        ) : (
          <span className="inline-flex shrink-0 items-center rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
            Not installed
          </span>
        )}
        <div className="flex shrink-0 items-center gap-1">
          <Button
            className="h-7 px-3 text-xs"
            data-testid={`custom-harness-edit-${entry.id}`}
            onClick={() => setEditing(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            Edit
          </Button>
          {confirmingDelete ? (
            <>
              <Button
                className="h-7 px-3 text-xs"
                data-testid={`custom-harness-delete-confirm-${entry.id}`}
                disabled={del.isPending || !confirmState.canConfirm}
                onClick={() => {
                  setDeleteError(null);
                  del.mutate(entry.id, {
                    onSuccess: () => setConfirmingDelete(false),
                    onError: (err) => {
                      setDeleteError(
                        err instanceof Error ? err.message : String(err),
                      );
                      // Keep confirmation open so user sees the error.
                    },
                  });
                }}
                size="sm"
                type="button"
                variant="destructive"
              >
                {del.isPending ? <Spinner className="h-3.5 w-3.5" /> : "Delete"}
              </Button>
              <Button
                className="h-7 px-3 text-xs"
                onClick={() => {
                  setConfirmingDelete(false);
                  setDeleteError(null);
                }}
                size="sm"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
            </>
          ) : (
            <button
              aria-label={`Delete ${entry.label}`}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
              data-testid={`custom-harness-delete-${entry.id}`}
              onClick={() => {
                setDeleteError(null);
                setConfirmingDelete(true);
              }}
              type="button"
            >
              <Trash2 className="h-4 w-4" />
            </button>
          )}
        </div>
      </div>
      {confirmingDelete ? (
        <p
          className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-sm text-amber-600 dark:text-amber-400"
          data-testid={`custom-harness-delete-warning-${entry.id}`}
        >
          {confirmState.message}
        </p>
      ) : null}
      {deleteError ? (
        <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
          {deleteError}
        </p>
      ) : null}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function HarnessManagementCard() {
  const runtimesQuery = useAcpRuntimesQuery();
  const catalog = runtimesQuery.data ?? [];
  const [showForm, setShowForm] = React.useState(false);
  const [presetPrefill, setPresetPrefill] = React.useState<
    Partial<CustomFormValues> | undefined
  >(undefined);

  // Preset entries come from the backend catalog; sort detected-first then
  // alphabetically within each group — via the tested harnessGalleryLogic helper.
  const presetEntries = React.useMemo(
    () => sortedPresetEntries(catalog),
    [catalog],
  );

  const customEntries = getCustomEntries(catalog);

  function handleFormClose() {
    setShowForm(false);
    setPresetPrefill(undefined);
  }

  return (
    <section
      className="min-w-0 space-y-4"
      data-testid="settings-harness-management"
    >
      <SettingsSectionHeader
        title="Bring your own harness"
        description="Register any ACP-speaking agent tool as a selectable runtime. Pick from presets or add a custom command."
      />

      {/* Preset gallery — driven from backend catalog, detected-first */}
      {presetEntries.length > 0 ? (
        <div className="space-y-3">
          <p className="text-sm font-medium">Presets</p>
          <div
            className="grid grid-cols-1 gap-3 sm:grid-cols-2"
            data-testid="harness-preset-gallery"
          >
            {presetEntries.map((entry) => (
              <PresetCard entry={entry} key={entry.id} />
            ))}
          </div>
        </div>
      ) : null}

      {/* Custom harnesses list */}
      {customEntries.length > 0 ? (
        <div className="space-y-3">
          <p className="text-sm font-medium">Custom harnesses</p>
          <div className="space-y-2" data-testid="custom-harness-list">
            {customEntries.map((entry) => (
              <CustomHarnessRow entry={entry} key={entry.id} />
            ))}
          </div>
        </div>
      ) : null}

      {/* Add custom / form toggle */}
      {showForm ? (
        <CustomHarnessForm
          initial={presetPrefill}
          onCancel={handleFormClose}
          onSaved={handleFormClose}
        />
      ) : (
        <Button
          className="gap-2"
          data-testid="harness-add-custom-button"
          onClick={() => {
            setPresetPrefill(undefined);
            setShowForm(true);
          }}
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="h-4 w-4" />
          Add custom harness
        </Button>
      )}

      {runtimesQuery.error instanceof Error ? (
        <p className="rounded-2xl bg-destructive/10 px-4 py-4 text-sm text-destructive">
          {runtimesQuery.error.message}
        </p>
      ) : null}
    </section>
  );
}
