import { AlertCircle, Lock, Plus, X } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./personaDialogPickers";

export type EnvVarsValue = Record<string, string>;

/**
 * Returns true when a required env key is unsatisfied — neither the agent-local
 * value nor the inherited (global / persona) value provides it.
 *
 * Used by `EnvVarsEditor` to render the amber "Required" badge on unfilled rows.
 * Exported for unit testing.
 */
export function isRequiredKeyMissing(
  key: string,
  localValue: EnvVarsValue,
  inheritedFrom: EnvVarsValue | undefined,
): boolean {
  const local = localValue[key] ?? "";
  const inherited = inheritedFrom?.[key] ?? "";
  return local.length === 0 && inherited.length === 0;
}

type EnvVarsEditorProps = {
  /** The current key/value map. */
  value: EnvVarsValue;
  /** Called with a new map whenever the user edits a row. */
  onChange: (next: EnvVarsValue) => void;
  /** Optional: shown as greyed-out hints next to rows whose key collides
   * with this map (e.g., a persona-set value for the same key). */
  inheritedFrom?: EnvVarsValue;
  /** Label for the inherited source (e.g., "persona"). */
  inheritedLabel?: string;
  /** Section header. Defaults to "Environment variables". */
  label?: string;
  /** Short description below the header. */
  helperText?: string;
  /** Disables all editing. */
  disabled?: boolean;
  /**
   * Env var keys that are required for the agent to start with the currently
   * selected runtime + provider. Each key renders as a locked first-class row
   * at the top of the editor: the key is pre-filled and read-only; the value
   * is editable. If the value is already set in `value`, it is shown; otherwise
   * the row is empty and marked with a "Required" badge so the user knows to
   * fill it in.
   */
  requiredKeys?: readonly string[];
  /**
   * Env var keys that are required but already satisfied by the runtime's
   * config file (e.g. `~/.config/goose/config.yaml`). These are shown as
   * read-only informational rows with a "Set in goose config" annotation so
   * the user knows the key is covered without needing to add it here.
   */
  fileSatisfiedKeys?: readonly string[];
};

type Row = { id: string; key: string; value: string };

/**
 * A flat key/value editor for environment variables.
 *
 * Maintains an ordered list of rows internally (so duplicate / empty keys
 * don't collapse mid-edit) and emits the latest non-empty rows as a record
 * via `onChange`. No validation, no warnings, no key shape enforcement —
 * by design.
 */
export function EnvVarsEditor({
  value,
  onChange,
  inheritedFrom,
  inheritedLabel = "inherited",
  label = "Environment variables",
  helperText,
  disabled = false,
  requiredKeys = [],
  fileSatisfiedKeys = [],
}: EnvVarsEditorProps) {
  // Local ordered row state. Synced from `value` on mount and when the
  // parent supplies a value we did NOT just emit (e.g., dialog reopened
  // with a different persona/agent). We track what we last emitted so a
  // row with an empty key doesn't get wiped: emit returns {} for it, the
  // parent's useState produces a new object reference, but `value` content
  // matches our `lastEmitted`, so we skip the resync.
  const [rows, setRows] = React.useState<Row[]>(() => toRows(value));
  const lastEmitted = React.useRef<EnvVarsValue>(toRecord(toRows(value)));
  React.useEffect(() => {
    if (!recordsEqual(lastEmitted.current, value)) {
      lastEmitted.current = value;
      setRows(toRows(value));
    }
  }, [value]);

  function emit(next: Row[]) {
    setRows(next);
    const record = toRecord(next);
    lastEmitted.current = record;
    onChange(record);
  }

  function updateRow(id: string, patch: Partial<Pick<Row, "key" | "value">>) {
    emit(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)));
  }

  function removeRow(id: string) {
    emit(rows.filter((row) => row.id !== id));
  }

  function addRow() {
    emit([...rows, { id: crypto.randomUUID(), key: "", value: "" }]);
  }

  // Required rows are rendered before the user-editable rows. They are not
  // part of `rows` state — they read from / write to `value` directly via
  // `onChange`, using their key as the stable identity.
  function updateRequiredValue(key: string, newValue: string) {
    onChange({ ...value, [key]: newValue });
  }

  return (
    <div className="space-y-2" data-testid="env-vars-editor">
      <div>
        <div className="text-sm font-medium">{label}</div>
        {helperText ? (
          <p className="mt-0.5 text-xs text-muted-foreground">{helperText}</p>
        ) : null}
      </div>
      <div className="space-y-2">
        {/* Required credential rows — shown first, key is read-only */}
        {requiredKeys.map((key) => {
          const currentValue = value[key] ?? "";
          // A required key is only "missing" if neither the agent-local value
          // nor the inherited (global / persona) value provides it.
          const isMissing = isRequiredKeyMissing(key, value, inheritedFrom);
          return (
            <div key={key} className="space-y-1">
              <div className="flex items-center gap-2">
                <div
                  className={cn(
                    "flex min-h-11 flex-1 items-center gap-1.5 px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                    "border-amber-500/40 bg-amber-50/30 dark:bg-amber-950/20",
                  )}
                >
                  <Lock
                    className="h-3 w-3 shrink-0 text-muted-foreground/60"
                    aria-hidden
                  />
                  <span
                    className="font-mono text-sm leading-6 text-foreground/80"
                    data-testid="env-vars-required-key"
                  >
                    {key}
                  </span>
                  {isMissing ? (
                    <span className="ml-1 flex items-center gap-0.5 rounded-sm bg-amber-100 px-1 py-0.5 text-2xs font-medium text-amber-700 dark:bg-amber-900/40 dark:text-amber-400">
                      <AlertCircle className="h-2.5 w-2.5" aria-hidden />
                      Required
                    </span>
                  ) : null}
                </div>
                <div
                  className={cn(
                    "flex min-h-11 flex-[2] items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label={`Value for ${key}`}
                    className={cn(
                      "h-8 px-0 py-0 font-mono leading-6",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    data-testid="env-vars-required-value"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRequiredValue(key, event.target.value)
                    }
                    placeholder="value"
                    type="password"
                    value={currentValue}
                  />
                </div>
                {/* Spacer to align with the remove-button column */}
                <div className="h-9 w-9 shrink-0" aria-hidden />
              </div>
              {(() => {
                const inheritedValue = inheritedFrom?.[key];
                return inheritedValue !== undefined ? (
                  <p className="ml-1 text-xs text-muted-foreground">
                    Overrides {inheritedLabel} value{" "}
                    <span className="font-mono">
                      {maskInherited(inheritedValue)}
                    </span>
                  </p>
                ) : null;
              })()}
            </div>
          );
        })}

        {/* File-satisfied keys — required but set in the runtime config file */}
        {fileSatisfiedKeys.map((key) => (
          <div key={key} className="space-y-1">
            <div className="flex items-center gap-2">
              <div
                className={cn(
                  "flex min-h-11 flex-1 items-center gap-1.5 px-3",
                  PERSONA_FIELD_SHELL_CLASS,
                  "border-muted-foreground/20 bg-muted/20",
                )}
              >
                <Lock
                  className="h-3 w-3 shrink-0 text-muted-foreground/40"
                  aria-hidden
                />
                <span
                  className="font-mono text-sm leading-6 text-foreground/60"
                  data-testid="env-vars-file-satisfied-key"
                >
                  {key}
                </span>
                <span className="ml-1 rounded-sm bg-muted px-1 py-0.5 text-2xs font-medium text-muted-foreground">
                  Set in goose config
                </span>
              </div>
              {/* Spacer columns to align with required-key rows */}
              <div
                className={cn(
                  "flex min-h-11 flex-[2] items-center px-3",
                  PERSONA_FIELD_SHELL_CLASS,
                  "opacity-40",
                )}
              >
                <span className="font-mono text-sm text-muted-foreground">
                  ••••••••
                </span>
              </div>
              <div className="h-9 w-9 shrink-0" aria-hidden />
            </div>
          </div>
        ))}

        {/* User-managed rows */}
        {rows.length === 0 &&
        requiredKeys.length === 0 &&
        fileSatisfiedKeys.length === 0 ? (
          <p className="text-xs italic text-muted-foreground">
            No variables set.
          </p>
        ) : null}
        {rows.map((row) => {
          const inheritedValue = inheritedFrom?.[row.key];
          const showsInherited =
            inheritedValue !== undefined && row.key.length > 0;
          return (
            <div key={row.id} className="space-y-1">
              <div className="flex items-center gap-2">
                <div
                  className={cn(
                    "flex min-h-11 flex-1 items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label="Variable name"
                    className={cn(
                      "h-8 px-0 py-0 font-mono leading-6",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    data-testid="env-vars-key"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { key: event.target.value })
                    }
                    placeholder="VARIABLE_NAME"
                    value={row.key}
                  />
                </div>
                <div
                  className={cn(
                    "flex min-h-11 flex-[2] items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label="Variable value"
                    className={cn(
                      "h-8 px-0 py-0 font-mono leading-6",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    data-testid="env-vars-value"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { value: event.target.value })
                    }
                    placeholder="value"
                    value={row.value}
                  />
                </div>
                <Button
                  aria-label="Remove variable"
                  data-testid="env-vars-remove"
                  disabled={disabled}
                  onClick={() => removeRow(row.id)}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
              {showsInherited ? (
                <p className="ml-1 text-xs text-muted-foreground">
                  Overrides {inheritedLabel} value{" "}
                  <span className="font-mono">
                    {maskInherited(inheritedValue)}
                  </span>
                </p>
              ) : null}
            </div>
          );
        })}
        <Button
          data-testid="env-vars-add"
          disabled={disabled}
          onClick={addRow}
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="mr-1 h-4 w-4" />
          Add variable
        </Button>
      </div>
    </div>
  );
}

/**
 * Render a masked preview of an inherited (persona) env value so the agent
 * dialog can show "Overrides persona value •••• (last 4)" without exposing
 * the persona's actual secret to anyone viewing the agent UI. Empty values
 * render as "(empty)" so the user can still tell the persona had a value
 * set at all.
 */
function maskInherited(value: string): string {
  if (value.length === 0) return "(empty)";
  if (value.length <= 4) return "•".repeat(value.length);
  return `••••${value.slice(-4)}`;
}

function toRows(value: EnvVarsValue): Row[] {
  return Object.entries(value).map(([key, val]) => ({
    id: crypto.randomUUID(),
    key,
    value: val,
  }));
}

function recordsEqual(a: EnvVarsValue, b: EnvVarsValue): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    // `in` walks the prototype, but EnvVarsValue is always a plain Record
    // built from `toRecord` (Object.create-less), so this is safe here.
    if (!(key in b)) return false;
    if (a[key] !== b[key]) return false;
  }
  return true;
}

function toRecord(rows: Row[]): EnvVarsValue {
  const out: EnvVarsValue = {};
  for (const row of rows) {
    // Empty key = user is mid-edit; skip it so we don't poison the record.
    // Duplicate keys: last write wins (matches Command::env semantics).
    if (row.key.length > 0) {
      out[row.key] = row.value;
    }
  }
  return out;
}
