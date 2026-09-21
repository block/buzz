import * as React from "react";
import { Plus, X } from "lucide-react";

import { useTranslation } from "@/i18n";
import {
  useManagedAgentPrereqsQuery,
  useSaveCustomHarnessMutation,
} from "@/features/agents/hooks";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "@/features/agents/ui/agentConfigOptions";
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
  const { t } = useTranslation();
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
      {available
        ? t("settings.custom-harness.found-on-path")
        : t("settings.custom-harness.not-found-on-path")}
    </span>
  );
}

// ── Shared field chrome (matches the Create agent dialog) ────────────────────

function FieldShell({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-h-11 items-center px-3",
        PERSONA_FIELD_SHELL_CLASS,
        className,
      )}
    >
      {children}
    </div>
  );
}

const FIELD_INPUT_CLASS = cn(
  "h-8 px-0 py-0 leading-6",
  PERSONA_FIELD_CONTROL_CLASS,
);

// ── Args / env editors ────────────────────────────────────────────────────────

function ArgsEditor({
  args,
  onChange,
}: {
  args: string[];
  onChange: (next: string[]) => void;
}) {
  const { t } = useTranslation();
  function set(index: number, value: string) {
    const next = [...args];
    next[index] = value;
    onChange(next);
  }

  return (
    <div className="space-y-2">
      {args.map((arg, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: positional arg list
        <div className="flex items-center gap-2" key={i}>
          <FieldShell className="flex-1">
            <Input
              className={cn(FIELD_INPUT_CLASS, "font-mono")}
              onChange={(e) => set(i, e.target.value)}
              placeholder={`arg ${i + 1}`}
              value={arg}
            />
          </FieldShell>
          <Button
            aria-label={t("settings.custom-harness.remove-arg-aria")}
            onClick={() => onChange(args.filter((_, idx) => idx !== i))}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      ))}
      <Button
        onClick={() => onChange([...args, ""])}
        size="sm"
        type="button"
        variant="outline"
      >
        <Plus className="mr-1 h-4 w-4" />
        {t("settings.custom-harness.add-arg")}
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
  const { t } = useTranslation();
  function set(index: number, field: "key" | "value", value: string) {
    onChange(env.map((e, i) => (i === index ? { ...e, [field]: value } : e)));
  }

  return (
    <div className="space-y-2">
      {env.map((pair, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: positional env list
        <div className="flex items-center gap-2" key={i}>
          <FieldShell className="flex-1">
            <Input
              className={cn(FIELD_INPUT_CLASS, "font-mono")}
              onChange={(e) => set(i, "key", e.target.value)}
              placeholder="KEY"
              value={pair.key}
            />
          </FieldShell>
          <FieldShell className="flex-2">
            <Input
              className={cn(FIELD_INPUT_CLASS, "font-mono")}
              onChange={(e) => set(i, "value", e.target.value)}
              placeholder="value"
              value={pair.value}
            />
          </FieldShell>
          <Button
            aria-label={t("settings.custom-harness.remove-env-aria")}
            onClick={() => onChange(env.filter((_, idx) => idx !== i))}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      ))}
      <Button
        onClick={() => onChange([...env, { key: "", value: "" }])}
        size="sm"
        type="button"
        variant="outline"
      >
        <Plus className="mr-1 h-4 w-4" />
        {t("settings.custom-harness.add-env")}
      </Button>
    </div>
  );
}

// ── Custom harness form ───────────────────────────────────────────────────────

/**
 * Create/edit form for a custom harness. Name and Command are the only
 * required fields — that's all a working ACP harness needs. ID (auto-derived
 * from name), arguments, env vars, docs URL, and install hint follow inline.
 */
export function CustomHarnessForm({
  initial,
  originalId,
  onCancel,
  onSaved,
  chromeless = false,
  header,
}: {
  initial?: Partial<CustomFormValues>;
  /** Id of the harness being edited, if this is an edit (not new). Used to
   * delete the old file when the id changes. */
  originalId?: string;
  onCancel: () => void;
  /** Receives the id the harness was saved under (the form may rewrite it). */
  onSaved: (id: string) => void;
  /** Render without the bordered card chrome (for embedding in the catalog
   * dialog detail pane). */
  chromeless?: boolean;
  /** Optional intro block rendered at the top of the scrollable body
   * (chromeless mode) so it scrolls with the fields. */
  header?: React.ReactNode;
}) {
  const { t } = useTranslation();
  const [form, setForm] = React.useState<CustomFormValues>({
    ...EMPTY_CUSTOM_FORM,
    ...initial,
  });
  const [error, setError] = React.useState<string | null>(null);
  const save = useSaveCustomHarnessMutation();

  // Save stays disabled until every required field is satisfied: the three
  // required text fields are non-blank and no args/env row is left empty.
  const requiredFieldsSatisfied =
    form.label.trim().length > 0 &&
    form.command.trim().length > 0 &&
    form.id.trim().length > 0 &&
    form.args.every((arg) => arg.trim().length > 0) &&
    form.env.every((pair) => pair.key.trim().length > 0);

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
      const definition = definitionFromFormValues(form);
      await save.mutateAsync({ definition, originalId });
      onSaved(definition.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <form
      className={cn(
        chromeless
          ? "flex min-h-0 flex-1 flex-col"
          : "space-y-5 rounded-2xl border border-border/60 bg-muted/10 px-4 py-4",
      )}
      data-testid="custom-harness-form"
      onSubmit={(e) => void handleSubmit(e)}
    >
      {chromeless ? null : (
        <div className="flex items-center justify-between">
          <p className="text-sm font-medium">
            {originalId
              ? t("settings.custom-harness.edit-title")
              : t("agents.harness.add-custom-title")}
          </p>
          <button
            aria-label={t("settings.common.cancel")}
            className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
            onClick={onCancel}
            type="button"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      <div
        className={cn(
          "space-y-5",
          chromeless && "min-h-0 flex-1 overflow-y-auto px-5 py-5",
        )}
      >
        {header}

        <div className="flex gap-3">
          <div className="flex-1 space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ch-label"
            >
              {t("settings.custom-harness.name")}
            </label>
            <FieldShell>
              <Input
                className={FIELD_INPUT_CLASS}
                id="ch-label"
                onChange={field("label")}
                placeholder="My Runtime"
                required
                value={form.label}
              />
            </FieldShell>
          </div>

          <div className="flex-1 space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ch-id"
            >
              ID
              <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                (auto-derived)
              </span>
            </label>
            <FieldShell>
              <Input
                className={cn(FIELD_INPUT_CLASS, "font-mono")}
                id="ch-id"
                onChange={field("id")}
                placeholder="my-runtime"
                required
                value={form.id}
              />
            </FieldShell>
          </div>
        </div>

        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ch-command"
            >
              {t("settings.custom-harness.command")}
            </label>
            <CommandAvailabilityBadge command={form.command} />
          </div>
          <FieldShell>
            <Input
              className={cn(FIELD_INPUT_CLASS, "font-mono")}
              id="ch-command"
              onChange={field("command")}
              placeholder="my-agent-bin"
              required
              value={form.command}
            />
          </FieldShell>
          <p className="text-xs text-muted-foreground">
            {t("settings.custom-harness.command-hint")}
          </p>
        </div>

        <div className="space-y-1.5">
          <p className="text-sm font-medium text-foreground">
            {t("settings.custom-harness.arguments")}
          </p>
          <ArgsEditor
            args={form.args}
            onChange={(args) => setForm((p) => ({ ...p, args }))}
          />
        </div>

        <div className="space-y-1.5">
          <p className="text-sm font-medium text-foreground">
            {t("settings.custom-harness.env-vars")}
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
              (override at spawn time; Buzz-managed vars always win)
            </span>
          </p>
          <EnvEditor
            env={form.env}
            onChange={(env) => setForm((p) => ({ ...p, env }))}
          />
        </div>

        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="ch-docs-url"
          >
            {t("settings.custom-harness.docs-url")}
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>(optional)</span>
          </label>
          <FieldShell>
            <Input
              className={FIELD_INPUT_CLASS}
              id="ch-docs-url"
              onChange={field("installInstructionsUrl")}
              placeholder="https://example.com/docs"
              value={form.installInstructionsUrl}
            />
          </FieldShell>
        </div>

        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="ch-install-hint"
          >
            {t("settings.custom-harness.install-hint")}
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>(optional)</span>
          </label>
          <FieldShell>
            <Input
              className={FIELD_INPUT_CLASS}
              id="ch-install-hint"
              onChange={field("installHint")}
              placeholder="npm install -g my-harness"
              value={form.installHint}
            />
          </FieldShell>
        </div>

        {error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
            {error}
          </p>
        ) : null}
      </div>

      <div
        className={cn(
          "flex justify-end gap-2",
          chromeless
            ? "shrink-0 border-t border-border/60 bg-background px-5 py-3"
            : "pt-1",
        )}
      >
        <Button
          disabled={save.isPending || !requiredFieldsSatisfied}
          type="submit"
        >
          {save.isPending ? <Spinner className="mr-2 h-3.5 w-3.5" /> : null}
          {t("settings.common.save")}
        </Button>
      </div>
    </form>
  );
}
