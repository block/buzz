import * as React from "react";
import { Plus, X } from "lucide-react";

import {
  useManagedAgentPrereqsQuery,
  useSaveCustomHarnessMutation,
} from "@/features/agents/hooks";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

import {
  commaArgError,
  type CustomFormValues,
  definitionFromFormValues,
  idFromLabel,
} from "./harnessFormLogic";

// ── Shared empty state ────────────────────────────────────────────────────────

export const EMPTY_CUSTOM_FORM: CustomFormValues = {
  id: "",
  label: "",
  command: "",
  args: [],
  env: [],
  installInstructionsUrl: "",
  installHint: "",
};

// ── Inline command validation ─────────────────────────────────────────────────

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

// ── Args / env editors ────────────────────────────────────────────────────────

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
            onClick={() => onChange(args.filter((_, idx) => idx !== i))}
            type="button"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <Button
        className="h-7 gap-1.5 px-3 text-xs"
        onClick={() => onChange([...args, ""])}
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
    onChange(env.map((e, i) => (i === index ? { ...e, [field]: value } : e)));
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
            onClick={() => onChange(env.filter((_, idx) => idx !== i))}
            type="button"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <Button
        className="h-7 gap-1.5 px-3 text-xs"
        onClick={() => onChange([...env, { key: "", value: "" }])}
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

// ── Progressive-disclosure custom form ────────────────────────────────────────

/**
 * Create/edit form for a custom harness.
 *
 * Progressive disclosure: only Name and Command are visible by default —
 * that's all a working ACP harness needs. ID (auto-derived from name),
 * arguments, env vars, docs URL, and install hint live behind an Advanced
 * disclosure. Editing an existing harness opens Advanced automatically when
 * any advanced field already has a value, so nothing is hidden from the user
 * mid-edit.
 */
export function CustomHarnessForm({
  initial,
  originalId,
  onCancel,
  onSaved,
  chromeless = false,
}: {
  initial?: Partial<CustomFormValues>;
  /** Id of the harness being edited, if this is an edit (not new). Used to
   * delete the old file when the id changes. */
  originalId?: string;
  onCancel: () => void;
  onSaved: () => void;
  /** Render without the bordered card chrome (for embedding in the catalog
   * dialog detail pane). */
  chromeless?: boolean;
}) {
  const [form, setForm] = React.useState<CustomFormValues>({
    ...EMPTY_CUSTOM_FORM,
    ...initial,
  });
  const hasAdvancedContent =
    form.args.length > 0 ||
    form.env.length > 0 ||
    form.installInstructionsUrl.trim().length > 0 ||
    form.installHint.trim().length > 0 ||
    (Boolean(originalId) && form.id !== idFromLabel(form.label));
  const [advancedOpen, setAdvancedOpen] = React.useState(hasAdvancedContent);
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
      setAdvancedOpen(true);
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
      className={cn(
        "space-y-3",
        !chromeless &&
          "rounded-2xl border border-border/60 bg-muted/10 px-4 py-4",
      )}
      data-testid="custom-harness-form"
      onSubmit={(e) => void handleSubmit(e)}
    >
      {chromeless ? null : (
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
      )}

      <div className="space-y-1">
        <p className="text-xs text-muted-foreground">Name</p>
        <Input
          className="h-8 text-sm"
          id="ch-label"
          onChange={field("label")}
          placeholder="My Harness"
          required
          value={form.label}
        />
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
        <p className="text-xs text-muted-foreground/70">
          Any command that speaks ACP over stdio works.
        </p>
      </div>

      <button
        aria-expanded={advancedOpen}
        className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
        data-testid="custom-harness-advanced-toggle"
        onClick={() => setAdvancedOpen((open) => !open)}
        type="button"
      >
        <ChevronRight
          className={cn(
            "h-3.5 w-3.5 transition-transform",
            advancedOpen && "rotate-90",
          )}
        />
        Advanced
      </button>

      {advancedOpen ? (
        <div
          className="space-y-3 border-l border-border/60 pl-3"
          data-testid="custom-harness-advanced"
        >
          <div className="space-y-1">
            <p className="text-xs text-muted-foreground">
              ID{" "}
              <span className="text-muted-foreground/60">(auto-derived)</span>
            </p>
            <Input
              className="h-8 font-mono text-sm"
              id="ch-id"
              onChange={field("id")}
              placeholder="my-harness"
              required
              value={form.id}
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
              Docs URL{" "}
              <span className="text-muted-foreground/60">(optional)</span>
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
        </div>
      ) : null}

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
