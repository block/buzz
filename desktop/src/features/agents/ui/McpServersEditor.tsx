/**
 * Editor for the local-only buzz-agent MCP server layer (Q1/Q3, PLANS/PR3).
 *
 * Modeled on `EnvVarsEditor`: an ordered row list keyed by stable ids (so
 * duplicate/empty names don't collapse mid-edit), inherited-layer read-only
 * rows that a same-named local row overrides or masks, and pure emit/resync
 * helpers exported for unit testing.
 *
 * STDIO only — no SSE/HTTP, no timeout, no headers (the backend doesn't
 * support them). Command and Args are separate fields (Q1): no combined
 * command string, no shell-quote dependency.
 */
import {
  AlertCircle,
  CheckCircle2,
  CircleSlash,
  Lock,
  Plus,
  X,
} from "lucide-react";
import * as React from "react";

import type { McpServerConfig } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { cn } from "@/shared/lib/cn";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./personaDialogPickers";

export type McpServersValue = McpServerConfig[];

/** Max byte length of a server name. Mirrors `MAX_SERVER_NAME_LEN` in `mcp_servers.rs`. */
export const MCP_SERVER_NAME_MAX_LEN = 128;
/** The bundled sidecar's reserved name. Mirrors `RESERVED_SERVER_NAME`. */
export const MCP_RESERVED_SERVER_NAME = "buzz-dev-mcp";
/** Effective/per-layer enabled-server cap. Mirrors `MAX_USER_MCP_SERVERS`. */
export const MAX_USER_MCP_SERVERS = 15;

/** Per-value byte cap. Mirrors `MAX_ENV_VALUE_BYTES` in `env_vars.rs`. */
export const MAX_ENV_VALUE_BYTES = 32 * 1024;

/**
 * Reserved env keys (case-insensitive). Mirrors `RESERVED_ENV_KEYS` in
 * `env_vars.rs`. Keys that override agent identity, code-execution surface,
 * security gates, or structured transport must never be user-settable.
 */
export const RESERVED_ENV_KEYS: readonly string[] = [
  "BUZZ_PRIVATE_KEY",
  "NOSTR_PRIVATE_KEY",
  "BUZZ_AUTH_TAG",
  "BUZZ_API_TOKEN",
  "BUZZ_ACP_PRIVATE_KEY",
  "BUZZ_ACP_API_TOKEN",
  "BUZZ_RELAY_URL",
  "BUZZ_ACP_AGENT_COMMAND",
  "BUZZ_ACP_AGENT_ARGS",
  "BUZZ_ACP_MCP_COMMAND",
  "BUZZ_ACP_MCP_SERVERS",
  "BUZZ_ACP_RESPOND_TO",
  "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
  "BUZZ_ACP_AGENT_OWNER",
  "BUZZ_ACP_SETUP_PAYLOAD",
];

const RESERVED_ENV_KEYS_UPPER = new Set(
  RESERVED_ENV_KEYS.map((k) => k.toUpperCase()),
);

const NAME_CHARS_RE = /^[A-Za-z0-9_-]*$/;
/** POSIX env key: `[A-Za-z_][A-Za-z0-9_]*`. Mirrors `is_well_formed_env_key`. */
const ENV_KEY_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

/**
 * Client-side mirror of the Rust `validate_mcp_servers` name grammar, for
 * inline feedback before a save round-trip. Returns `null` when valid.
 * Exported for unit tests.
 */
export function validateMcpServerName(name: string): string | null {
  if (name.length === 0) return "Name is required.";
  if (name.length > MCP_SERVER_NAME_MAX_LEN) {
    return `Name exceeds the maximum length (${MCP_SERVER_NAME_MAX_LEN} bytes).`;
  }
  if (!NAME_CHARS_RE.test(name)) {
    return "Only letters, numbers, `_`, and `-` are allowed.";
  }
  if (name.includes("__")) return "Name cannot contain `__`.";
  if (name === MCP_RESERVED_SERVER_NAME) {
    return `"${MCP_RESERVED_SERVER_NAME}" is reserved.`;
  }
  return null;
}

/**
 * Full per-row validation: name grammar + uniqueness within the layer +
 * command-required-when-enabled. `otherNames` is every OTHER row's name in
 * the same layer (excluding this row), so a collision flags both rows.
 * Exported for unit tests.
 */
export function validateMcpServerRow(
  row: { name: string; command: string; enabled: boolean },
  otherNames: ReadonlySet<string>,
): string | null {
  const nameError = validateMcpServerName(row.name);
  if (nameError) return nameError;
  if (otherNames.has(row.name)) return "Server names must be unique.";
  if (row.enabled && row.command.trim().length === 0) {
    return "Command is required for an enabled server.";
  }
  return null;
}

/**
 * Validate a single per-server env var entry. Mirrors the Rust
 * `validate_user_env_keys` boundary (`env_vars.rs`): POSIX key format,
 * reserved-key check (case-insensitive), NUL-free values, and per-value
 * byte cap. Returns `null` when valid. Exported for unit tests.
 */
export function validateMcpServerEnvEntry(entry: {
  name: string;
  value: string;
}): string | null {
  if (entry.name.length === 0) return null; // blank rows are skipped by toServers
  if (!ENV_KEY_RE.test(entry.name)) {
    return "Key must match [A-Za-z_][A-Za-z0-9_]*.";
  }
  if (RESERVED_ENV_KEYS_UPPER.has(entry.name.toUpperCase())) {
    return `"${entry.name}" is reserved by Buzz.`;
  }
  if (entry.value.includes("\0")) {
    return "Value cannot contain NUL bytes.";
  }
  if (new TextEncoder().encode(entry.value).length > MAX_ENV_VALUE_BYTES) {
    return `Value exceeds the ${MAX_ENV_VALUE_BYTES}-byte limit.`;
  }
  return null;
}

type ArgRow = { id: string; value: string };
type EnvRow = { id: string; name: string; value: string };
type ServerRow = {
  id: string;
  name: string;
  command: string;
  args: ArgRow[];
  env: EnvRow[];
  enabled: boolean;
};

/** Build a rows array from a server-config list. Exported for unit tests. */
export function toRows(servers: McpServersValue): ServerRow[] {
  return servers.map((server) => ({
    id: crypto.randomUUID(),
    name: server.name,
    command: server.command,
    args: server.args.map((value) => ({ id: crypto.randomUUID(), value })),
    env: server.env.map((entry) => ({
      id: crypto.randomUUID(),
      name: entry.name,
      value: entry.value,
    })),
    enabled: server.enabled,
  }));
}

/**
 * Collapse rows back to a server-config list. Rows with an empty name are
 * mid-edit (the user hasn't identified the server yet) and are skipped,
 * mirroring `EnvVarsEditor`'s empty-key skip. Blank arg/env rows are skipped
 * too — an unfilled "Add argument" row must never inject an empty-string
 * arg into the spawned command. Duplicate env var names within a server
 * collapse last-write-wins, matching `EnvVarsEditor`'s `toRecord` (its
 * object-keyed shape does this automatically; the per-server `env` here is
 * an array per the Rust `Vec<McpServerEnvVar>` shape, so it needs the same
 * dedupe applied explicitly to avoid a save-time rejection from the Rust
 * validator's duplicate-env-var check). Exported for unit tests.
 */
export function toServers(rows: readonly ServerRow[]): McpServersValue {
  const out: McpServerConfig[] = [];
  for (const row of rows) {
    if (row.name.length === 0) continue;
    const env = new Map<string, string>();
    for (const entry of row.env) {
      if (entry.name.length > 0) env.set(entry.name, entry.value);
    }
    out.push({
      name: row.name,
      command: row.command,
      args: row.args.map((a) => a.value).filter((v) => v.length > 0),
      env: Array.from(env, ([name, value]) => ({ name, value })),
      enabled: row.enabled,
    });
  }
  return out;
}

/** True iff two server lists are element-wise identical. Exported for unit tests. */
export function serversEqual(a: McpServersValue, b: McpServersValue): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.name !== y.name ||
      x.command !== y.command ||
      x.enabled !== y.enabled
    ) {
      return false;
    }
    if (
      x.args.length !== y.args.length ||
      x.args.some((v, j) => v !== y.args[j])
    ) {
      return false;
    }
    if (x.env.length !== y.env.length) return false;
    for (let j = 0; j < x.env.length; j++) {
      if (
        x.env[j].name !== y.env[j].name ||
        x.env[j].value !== y.env[j].value
      ) {
        return false;
      }
    }
  }
  return true;
}

/**
 * Merge server layers by name — later layers replace earlier entries whole,
 * matching the Rust `merge_mcp_servers` replace-by-name rule. Unlike the
 * Rust helper this keeps disabled entries and does not enforce the
 * effective-server cap: it is a DISPLAY merge for the editor's read-only
 * "inherited" rows, not the runtime WYSIWYG surface (that's
 * `RuntimeConfigSurface.buzzAgentMcpServers`, computed server-side). Sorted
 * by ASCII byte order (`<` / `>`) to match Rust's `BTreeMap<String, _>`
 * ordering. Exported for unit tests and dialog callers.
 */
export function mergeMcpServersByName(
  ...layers: readonly McpServersValue[]
): McpServersValue {
  const merged = new Map<string, McpServerConfig>();
  for (const layer of layers) {
    for (const server of layer) {
      merged.set(server.name, server);
    }
  }
  return Array.from(merged.values()).sort((a, b) =>
    a.name < b.name ? -1 : a.name > b.name ? 1 : 0,
  );
}

/**
 * Compute the effective enabled count across local and inherited layers.
 * Local enabled rows plus inherited enabled rows NOT overridden/masked by
 * a local row with the same name. Mirrors the Rust effective-merge cap
 * check (`mcp_servers.rs:225-229`). Exported for unit tests.
 */
export function effectiveEnabledCount(
  local: McpServersValue,
  inherited: McpServersValue,
): number {
  const localNames = new Set(local.map((s) => s.name));
  const localEnabled = local.filter((s) => s.enabled).length;
  const inheritedEnabled = inherited.filter(
    (s) => s.enabled && !localNames.has(s.name),
  ).length;
  return localEnabled + inheritedEnabled;
}

/** True when a row has any content beyond a bare name — used to decide
 * whether an empty-name row's "Name is required" hint should show (a
 * pristine just-added row stays quiet; a row with typed content but no name
 * yet warns, so filling in command/args/env doesn't silently go unsaved). */
function rowHasContent(row: ServerRow): boolean {
  return (
    row.command.trim().length > 0 ||
    row.args.some((a) => a.value.trim().length > 0) ||
    row.env.some(
      (e) => e.name.trim().length > 0 || e.value.trim().length > 0,
    ) ||
    !row.enabled
  );
}

function commandLine(command: string, args: readonly string[]): string {
  return [command, ...args].filter(Boolean).join(" ");
}

export type McpServersEditorProps = {
  /** This layer's own MCP server list. */
  value: McpServersValue;
  /** Called with the next list whenever the user edits a row. */
  onChange: (next: McpServersValue) => void;
  /** Read-only servers inherited from lower layer(s), already merged
   * (see `mergeMcpServersByName`). A local row with a matching name
   * overrides (or, if disabled, masks) the inherited entry. */
  inheritedServers?: McpServersValue;
  /** Label for the inherited source (e.g. "global" or "global + persona"). */
  inheritedLabel?: string;
  /** Section header. Defaults to "MCP servers". */
  label?: string;
  /** Short description below the header. */
  helperText?: string;
  /** Disables all editing. */
  disabled?: boolean;
};

/**
 * Per-server row editor: Name / Command / Args (repeatable) / Env
 * (repeatable) / Enabled toggle, plus read-only inherited-layer rows.
 */
export function McpServersEditor({
  value,
  onChange,
  inheritedServers = [],
  inheritedLabel = "inherited",
  label = "MCP servers",
  helperText,
  disabled = false,
}: McpServersEditorProps) {
  // Defense-in-depth: coerce at the exported boundary so a caller passing
  // `undefined` (e.g. a partial test bridge seed) degrades to empty rather
  // than crashing inside `toRows`. Production never produces `undefined`
  // (Rust `#[serde(default)]` + `EMPTY_CONFIG`), but a single coercion
  // here is cheaper than scattering `?? []` across every mount site.
  const safeValue = value ?? [];
  const [rows, setRows] = React.useState<ServerRow[]>(() => toRows(safeValue));
  const lastEmitted = React.useRef<McpServersValue>(safeValue);

  // Resync from `value` when the parent supplies a value we did not just
  // emit (e.g. dialog reopened against a different persona/agent). Mirrors
  // EnvVarsEditor's `recordsEqual` resync guard.
  React.useEffect(() => {
    if (!serversEqual(lastEmitted.current, safeValue)) {
      lastEmitted.current = safeValue;
      setRows(toRows(safeValue));
    }
  }, [safeValue]);

  function emit(nextRows: ServerRow[]) {
    setRows(nextRows);
    const servers = toServers(nextRows);
    lastEmitted.current = servers;
    onChange(servers);
  }

  function updateRow(id: string, patch: Partial<ServerRow>) {
    emit(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)));
  }

  function removeRow(id: string) {
    emit(rows.filter((row) => row.id !== id));
  }

  function addRow(seed?: Partial<Pick<ServerRow, "name" | "enabled">>) {
    emit([
      ...rows,
      {
        id: crypto.randomUUID(),
        name: seed?.name ?? "",
        command: "",
        args: [],
        env: [],
        enabled: seed?.enabled ?? true,
      },
    ]);
  }

  function addArg(rowId: string) {
    const row = rows.find((r) => r.id === rowId);
    if (!row) return;
    updateRow(rowId, {
      args: [...row.args, { id: crypto.randomUUID(), value: "" }],
    });
  }

  function updateArg(rowId: string, argId: string, argValue: string) {
    const row = rows.find((r) => r.id === rowId);
    if (!row) return;
    updateRow(rowId, {
      args: row.args.map((a) =>
        a.id === argId ? { ...a, value: argValue } : a,
      ),
    });
  }

  function removeArg(rowId: string, argId: string) {
    const row = rows.find((r) => r.id === rowId);
    if (!row) return;
    updateRow(rowId, { args: row.args.filter((a) => a.id !== argId) });
  }

  function addEnvRow(rowId: string) {
    const row = rows.find((r) => r.id === rowId);
    if (!row) return;
    updateRow(rowId, {
      env: [...row.env, { id: crypto.randomUUID(), name: "", value: "" }],
    });
  }

  function updateEnvRow(
    rowId: string,
    envId: string,
    patch: Partial<Pick<EnvRow, "name" | "value">>,
  ) {
    const row = rows.find((r) => r.id === rowId);
    if (!row) return;
    updateRow(rowId, {
      env: row.env.map((e) => (e.id === envId ? { ...e, ...patch } : e)),
    });
  }

  function removeEnvRow(rowId: string, envId: string) {
    const row = rows.find((r) => r.id === rowId);
    if (!row) return;
    updateRow(rowId, { env: row.env.filter((e) => e.id !== envId) });
  }

  const localNames = new Set(rows.map((r) => r.name));
  const visibleInherited = inheritedServers.filter(
    (server) => !localNames.has(server.name),
  );
  const localEnabledCount = rows.filter((r) => r.enabled).length;
  const inheritedEnabledCount = visibleInherited.filter(
    (s) => s.enabled,
  ).length;
  const atCap =
    localEnabledCount + inheritedEnabledCount >= MAX_USER_MCP_SERVERS;

  return (
    <div className="space-y-2" data-testid="mcp-servers-editor">
      <div>
        <div className="text-sm font-medium">{label}</div>
        {helperText ? (
          <p className="mt-0.5 text-xs text-muted-foreground">{helperText}</p>
        ) : null}
      </div>
      <div className="space-y-3">
        {visibleInherited.map((server) => (
          <InheritedServerRow
            inheritedLabel={inheritedLabel}
            key={server.name}
            onMask={() => addRow({ name: server.name, enabled: false })}
            disabled={disabled}
            server={server}
          />
        ))}

        {rows.length === 0 && visibleInherited.length === 0 ? (
          <p className="text-xs italic text-muted-foreground">
            No MCP servers configured.
          </p>
        ) : null}

        {rows.map((row) => {
          const otherNames = new Set(
            rows.filter((r) => r.id !== row.id).map((r) => r.name),
          );
          const rowError = validateMcpServerRow(row, otherNames);
          const showError = Boolean(
            rowError && (row.name.length > 0 || rowHasContent(row)),
          );
          const overridesInherited = inheritedServers.find(
            (s) => s.name === row.name,
          );

          return (
            <div
              className="space-y-2 rounded-xl border border-border/60 p-3"
              data-testid="mcp-servers-row"
              key={row.id}
            >
              <div className="flex items-center gap-2">
                <div
                  className={cn(
                    "flex min-h-11 flex-1 items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label="Server name"
                    className={cn(
                      "h-8 px-0 py-0 font-mono leading-6",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    data-testid="mcp-servers-name"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { name: event.target.value })
                    }
                    placeholder="server-name"
                    value={row.name}
                  />
                </div>
                <div
                  className={cn(
                    "flex min-h-11 flex-[2] items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label="Command"
                    className={cn(
                      "h-8 px-0 py-0 font-mono leading-6",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    data-testid="mcp-servers-command"
                    disabled={disabled}
                    onChange={(event) =>
                      updateRow(row.id, { command: event.target.value })
                    }
                    placeholder="npx"
                    value={row.command}
                  />
                </div>
                <Switch
                  aria-label={row.enabled ? "Disable server" : "Enable server"}
                  checked={row.enabled}
                  data-testid="mcp-servers-enabled"
                  disabled={disabled || (!row.enabled && atCap)}
                  onCheckedChange={(checked) =>
                    updateRow(row.id, { enabled: checked })
                  }
                />
                <Button
                  aria-label="Remove server"
                  data-testid="mcp-servers-remove"
                  disabled={disabled}
                  onClick={() => removeRow(row.id)}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>

              {showError ? (
                <p
                  className="ml-1 flex items-center gap-1 text-xs text-destructive"
                  data-testid="mcp-servers-error"
                >
                  <AlertCircle className="h-3 w-3 shrink-0" aria-hidden />
                  {rowError}
                </p>
              ) : null}
              {!showError && overridesInherited ? (
                <p className="ml-1 text-xs text-muted-foreground">
                  {row.enabled ? "Overrides" : "Masks"} {inheritedLabel} server{" "}
                  <span className="font-mono">
                    {commandLine(
                      overridesInherited.command,
                      overridesInherited.args,
                    ) || "stdio"}
                  </span>
                </p>
              ) : null}

              {/* Args */}
              <div className="ml-1 space-y-1.5">
                <p className="text-xs font-medium text-muted-foreground">
                  Args
                </p>
                {row.args.map((arg) => (
                  <div className="flex items-center gap-2" key={arg.id}>
                    <div
                      className={cn(
                        "flex min-h-9 flex-1 items-center px-3",
                        PERSONA_FIELD_SHELL_CLASS,
                      )}
                    >
                      <Input
                        aria-label="Argument"
                        className={cn(
                          "h-7 px-0 py-0 font-mono text-xs leading-6",
                          PERSONA_FIELD_CONTROL_CLASS,
                        )}
                        data-testid="mcp-servers-arg"
                        disabled={disabled}
                        onChange={(event) =>
                          updateArg(row.id, arg.id, event.target.value)
                        }
                        value={arg.value}
                      />
                    </div>
                    <Button
                      aria-label="Remove argument"
                      data-testid="mcp-servers-arg-remove"
                      disabled={disabled}
                      onClick={() => removeArg(row.id, arg.id)}
                      size="icon"
                      type="button"
                      variant="ghost"
                    >
                      <X className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                ))}
                <Button
                  data-testid="mcp-servers-arg-add"
                  disabled={disabled}
                  onClick={() => addArg(row.id)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <Plus className="mr-1 h-3.5 w-3.5" />
                  Add argument
                </Button>
              </div>

              {/* Env */}
              <div className="ml-1 space-y-1.5">
                <p className="text-xs font-medium text-muted-foreground">
                  Environment
                </p>
                {row.env.map((entry) => {
                  const envError = validateMcpServerEnvEntry(entry);
                  return (
                    <div key={entry.id}>
                      <div className="flex items-center gap-2">
                        <div
                          className={cn(
                            "flex min-h-9 flex-1 items-center px-3",
                            PERSONA_FIELD_SHELL_CLASS,
                          )}
                        >
                          <Input
                            aria-label="Env var name"
                            className={cn(
                              "h-7 px-0 py-0 font-mono text-xs leading-6",
                              PERSONA_FIELD_CONTROL_CLASS,
                            )}
                            data-testid="mcp-servers-env-name"
                            disabled={disabled}
                            onChange={(event) =>
                              updateEnvRow(row.id, entry.id, {
                                name: event.target.value,
                              })
                            }
                            placeholder="VARIABLE_NAME"
                            value={entry.name}
                          />
                        </div>
                        <div
                          className={cn(
                            "flex min-h-9 flex-[2] items-center px-3",
                            PERSONA_FIELD_SHELL_CLASS,
                          )}
                        >
                          <Input
                            aria-label="Env var value"
                            className={cn(
                              "h-7 px-0 py-0 font-mono text-xs leading-6",
                              PERSONA_FIELD_CONTROL_CLASS,
                            )}
                            data-testid="mcp-servers-env-value"
                            disabled={disabled}
                            onChange={(event) =>
                              updateEnvRow(row.id, entry.id, {
                                value: event.target.value,
                              })
                            }
                            placeholder="value"
                            value={entry.value}
                          />
                        </div>
                        <Button
                          aria-label="Remove env var"
                          data-testid="mcp-servers-env-remove"
                          disabled={disabled}
                          onClick={() => removeEnvRow(row.id, entry.id)}
                          size="icon"
                          type="button"
                          variant="ghost"
                        >
                          <X className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                      {envError ? (
                        <p
                          className="ml-1 mt-0.5 flex items-center gap-1 text-xs text-destructive"
                          data-testid="mcp-servers-env-error"
                        >
                          <AlertCircle
                            className="h-3 w-3 shrink-0"
                            aria-hidden
                          />
                          {envError}
                        </p>
                      ) : null}
                    </div>
                  );
                })}
                <Button
                  data-testid="mcp-servers-env-add"
                  disabled={disabled}
                  onClick={() => addEnvRow(row.id)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <Plus className="mr-1 h-3.5 w-3.5" />
                  Add variable
                </Button>
              </div>
            </div>
          );
        })}

        <div className="space-y-1">
          <Button
            data-testid="mcp-servers-add"
            disabled={disabled || atCap}
            onClick={() => addRow()}
            size="sm"
            type="button"
            variant="outline"
          >
            <Plus className="mr-1 h-4 w-4" />
            Add server
          </Button>
          {atCap ? (
            <p className="ml-1 text-xs text-muted-foreground">
              {MAX_USER_MCP_SERVERS}-server limit reached (including inherited).
            </p>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function InheritedServerRow({
  disabled,
  inheritedLabel,
  onMask,
  server,
}: {
  disabled: boolean;
  inheritedLabel: string;
  onMask: () => void;
  server: McpServerConfig;
}) {
  const StatusIcon = server.enabled ? CheckCircle2 : CircleSlash;
  const line = commandLine(server.command, server.args);

  return (
    <div
      className="flex items-center gap-2"
      data-testid="mcp-servers-inherited-row"
    >
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
        <StatusIcon
          className={cn(
            "h-3.5 w-3.5 shrink-0",
            server.enabled ? "text-emerald-600" : "text-muted-foreground",
          )}
          aria-hidden
        />
        <span className="min-w-0 flex-1 truncate">
          <span className="font-mono text-sm leading-6 text-foreground/70">
            {server.name}
          </span>
          <span className="ml-1 rounded-sm bg-muted px-1 py-0.5 text-2xs font-medium text-muted-foreground">
            Inherited from {inheritedLabel}
          </span>
          {line ? (
            <span
              className="ml-1.5 truncate font-mono text-xs text-muted-foreground/70"
              title={line}
            >
              {line}
            </span>
          ) : null}
        </span>
      </div>
      <Button
        data-testid="mcp-servers-mask"
        disabled={disabled}
        onClick={onMask}
        size="sm"
        type="button"
        variant="ghost"
      >
        Mask
      </Button>
    </div>
  );
}
