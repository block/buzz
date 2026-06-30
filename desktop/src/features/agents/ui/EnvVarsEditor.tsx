import { Plus, X } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./personaDialogPickers";

export type EnvVarsValue = Record<string, string>;

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

  return (
    <div className="space-y-2" data-testid="env-vars-editor">
      <div>
        <div className="text-sm font-medium">{label}</div>
        {helperText ? (
          <p className="mt-0.5 text-xs text-muted-foreground">{helperText}</p>
        ) : null}
      </div>
      <div className="space-y-2">
        {rows.length === 0 ? (
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
