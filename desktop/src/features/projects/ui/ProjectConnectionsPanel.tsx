import {
  ChevronDown,
  KeyRound,
  Link2,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  Wrench,
  XCircle,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  useCreateProjectConnectionMutation,
  useDeleteProjectConnectionMutation,
  useProjectConnectionsQuery,
  useTestProjectConnectionMutation,
  useUpdateProjectConnectionMutation,
} from "@/features/projects/projectConnectionHooks";
import type {
  ProjectConnection,
  ProjectConnectionDraft,
  ProjectConnectionHealthStatus,
} from "@/shared/api/tauriProjectConnections";
import type { ProjectConnectionScope } from "@/shared/api/projectConnectionTypes";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  PROJECT_DETAIL_PANEL_CLASS,
  PROJECT_DETAIL_PANEL_MESSAGE_CLASS,
  PROJECT_PANEL_ACTION_BUTTON_CLASS,
} from "./projectPanelStyles";
import { buildProjectConnectionSecretChanges } from "./projectConnectionSecrets";

const HEALTH_COPY: Record<
  ProjectConnectionHealthStatus,
  { label: string; className: string }
> = {
  ready: {
    label: "Tools found",
    className: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  },
  not_tested: {
    label: "Not tested",
    className: "bg-muted text-muted-foreground",
  },
  check_needed: {
    label: "Check needed",
    className: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
  },
  approval_required: {
    label: "Approval required",
    className: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
  },
  sign_in_required: {
    label: "Sign-in required",
    className: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
  },
  missing_access: {
    label: "Missing access",
    className: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
  },
  unavailable: {
    label: "Unavailable",
    className: "bg-destructive/10 text-destructive",
  },
};

type SecretRow = { id: string; key: string; value: string };
const TOOL_PREVIEW_LIMIT = 4;
const MAX_NAME_BYTES = 128;
const MAX_PROVIDER_BYTES = 64;
const MAX_COMMAND_BYTES = 1024;
const MAX_ARGS = 128;
const MAX_ARG_BYTES = 4096;

function utf8ByteLength(value: string) {
  return new TextEncoder().encode(value).byteLength;
}

function emptySecretRow(): SecretRow {
  return { id: crypto.randomUUID(), key: "", value: "" };
}

function toolLabel(tool: string) {
  return tool
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function connectionNeedsEditing(connection: ProjectConnection) {
  return (
    connection.health.status === "sign_in_required" ||
    connection.health.status === "missing_access" ||
    connection.health.status === "approval_required" ||
    (connection.health.status === "unavailable" &&
      connection.health.detail === "Buzz could not start this MCP server.")
  );
}

function connectionActionLabel(connection: ProjectConnection) {
  if (connection.health.status === "sign_in_required") {
    return "Update sign-in";
  }
  if (connection.health.status === "missing_access") {
    return "Update credentials";
  }
  if (connection.health.status === "approval_required") {
    return "Review command";
  }
  if (
    connection.health.status === "unavailable" &&
    connection.health.detail === "Buzz could not start this MCP server."
  ) {
    return "Review setup";
  }
  return connection.health.status === "not_tested" ? "Test" : "Test again";
}

function formatVerificationTime(timestamp: string | null) {
  if (!timestamp) return "Never tested";
  const date = new Date(timestamp);
  return Number.isNaN(date.valueOf())
    ? "Last test unavailable"
    : `Tested ${date.toLocaleString()}`;
}

function ConnectionHealthBadge({
  status,
}: {
  status: ProjectConnectionHealthStatus;
}) {
  const copy = HEALTH_COPY[status];
  return (
    <span
      className={`inline-flex items-center rounded-full px-2 py-0.5 text-2xs font-medium ${copy.className}`}
    >
      {copy.label}
    </span>
  );
}

function ConnectionDialog({
  connection,
  onOpenChange,
  onSave,
  onTest,
  open,
  pending,
  projectScope,
}: {
  connection: ProjectConnection | null;
  onOpenChange: (open: boolean) => void;
  onSave: (
    input: ProjectConnectionDraft & { id?: string },
  ) => Promise<ProjectConnection>;
  onTest: (connection: ProjectConnection) => Promise<void>;
  open: boolean;
  pending: boolean;
  projectScope: ProjectConnectionScope;
}) {
  const [name, setName] = React.useState("");
  const [provider, setProvider] = React.useState("");
  const [command, setCommand] = React.useState("");
  const [argsText, setArgsText] = React.useState("");
  const [showTechnicalDetails, setShowTechnicalDetails] = React.useState(false);
  const [secrets, setSecrets] = React.useState<SecretRow[]>([]);
  const [removedEnvKeys, setRemovedEnvKeys] = React.useState<string[]>([]);
  const [trusted, setTrusted] = React.useState(false);
  const [executionDirty, setExecutionDirty] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) {
      setName("");
      setProvider("");
      setCommand("");
      setArgsText("");
      setSecrets([]);
      setRemovedEnvKeys([]);
      setTrusted(false);
      setExecutionDirty(false);
      setError(null);
      return;
    }
    setName(connection?.name ?? "");
    setProvider(connection?.provider ?? "");
    setCommand(connection?.command ?? "");
    setArgsText(connection?.args.join("\n") ?? "");
    setSecrets(
      connection?.envKeys.map((key) => ({
        id: crypto.randomUUID(),
        key,
        value: "",
      })) ?? [emptySecretRow()],
    );
    setRemovedEnvKeys([]);
    setShowTechnicalDetails(Boolean(connection));
    setTrusted(false);
    setExecutionDirty(!connection || connectionNeedsEditing(connection));
    setError(null);
  }, [connection, open]);

  const requiresApproval = !connection || executionDirty;
  const parsedArgs = argsText
    .split("\n")
    .map((arg) => arg.trim())
    .filter(Boolean);
  const nameError =
    name.trim() && utf8ByteLength(name.trim()) > MAX_NAME_BYTES
      ? `Keep the connection name to ${MAX_NAME_BYTES} bytes or fewer.`
      : null;
  const providerError =
    provider.trim() && utf8ByteLength(provider.trim()) > MAX_PROVIDER_BYTES
      ? `Keep the service name to ${MAX_PROVIDER_BYTES} bytes or fewer.`
      : null;
  const commandError =
    command.trim() && utf8ByteLength(command.trim()) > MAX_COMMAND_BYTES
      ? `Keep the command to ${MAX_COMMAND_BYTES} bytes or fewer.`
      : null;
  const argsError =
    parsedArgs.length > MAX_ARGS ||
    parsedArgs.some((arg) => utf8ByteLength(arg) > MAX_ARG_BYTES)
      ? `Use no more than ${MAX_ARGS} arguments, with each ${MAX_ARG_BYTES} bytes or fewer.`
      : null;
  const hasValidFields = Boolean(
    name.trim() &&
      provider.trim() &&
      command.trim() &&
      !nameError &&
      !providerError &&
      !commandError &&
      !argsError,
  );
  const secretChanges = buildProjectConnectionSecretChanges(
    secrets,
    connection?.envKeys ?? [],
    removedEnvKeys,
  );
  const canSubmit =
    !pending &&
    hasValidFields &&
    secretChanges.ok &&
    (!requiresApproval || trusted);

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    setError(null);
    try {
      const saved = await onSave({
        ...(connection ? { id: connection.id } : {}),
        projectScope,
        name: name.trim(),
        provider: provider.trim(),
        command: command.trim(),
        args: parsedArgs,
        env: secretChanges.env,
        removeEnvKeys: secretChanges.removeEnvKeys,
        executionAcknowledged: requiresApproval ? trusted : false,
      });
      onOpenChange(false);
      if (requiresApproval) {
        await onTest(saved);
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Couldn't save this connection. Check the details and try again.",
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!pending) onOpenChange(nextOpen);
      }}
      open={open}
    >
      <DialogContent className="max-w-xl" showCloseButton={!pending}>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>
              {connection?.name ?? "Add Project connection"}
            </DialogTitle>
            <DialogDescription>
              {connection
                ? "Edit this Project connection. It applies across every repository in the Project; its credentials stay on this device."
                : "Connect an MCP server to this Project. The connection applies across every repository; its credentials stay on this device."}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-5">
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="connection-name"
              >
                Connection name <span aria-hidden="true">*</span>
              </label>
              <Input
                aria-describedby={
                  nameError ? "connection-name-error" : undefined
                }
                aria-invalid={Boolean(nameError) || undefined}
                disabled={pending}
                id="connection-name"
                maxLength={128}
                onChange={(event) => setName(event.target.value)}
                placeholder="Analytics"
                required
                value={name}
              />
              {nameError ? (
                <p
                  className="text-xs text-destructive"
                  id="connection-name-error"
                >
                  {nameError}
                </p>
              ) : null}
            </div>
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="connection-provider"
              >
                Service <span aria-hidden="true">*</span>
              </label>
              <Input
                aria-describedby={
                  providerError ? "connection-provider-error" : undefined
                }
                aria-invalid={Boolean(providerError) || undefined}
                disabled={pending}
                id="connection-provider"
                maxLength={64}
                onChange={(event) => setProvider(event.target.value)}
                placeholder="Google Analytics"
                required
                value={provider}
              />
              {providerError ? (
                <p
                  className="text-xs text-destructive"
                  id="connection-provider-error"
                >
                  {providerError}
                </p>
              ) : null}
            </div>
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="connection-command"
              >
                Connection command <span aria-hidden="true">*</span>
              </label>
              <Input
                autoCapitalize="off"
                autoCorrect="off"
                disabled={pending}
                aria-describedby={
                  commandError
                    ? "connection-command-help connection-command-error"
                    : "connection-command-help"
                }
                aria-invalid={Boolean(commandError) || undefined}
                id="connection-command"
                maxLength={1024}
                onChange={(event) => {
                  setCommand(event.target.value);
                  setExecutionDirty(true);
                  setTrusted(false);
                }}
                placeholder="/absolute/path/to/mcp-server"
                required
                spellCheck={false}
                value={command}
              />
              <p
                className="text-xs text-muted-foreground"
                id="connection-command-help"
              >
                Enter the executable's absolute path. Buzz runs it directly
                without a shell.
              </p>
              {commandError ? (
                <p
                  className="text-xs text-destructive"
                  id="connection-command-error"
                >
                  {commandError}
                </p>
              ) : null}
            </div>
            <button
              aria-expanded={showTechnicalDetails}
              className="inline-flex h-8 items-center gap-1.5 text-sm font-medium text-foreground"
              onClick={() => setShowTechnicalDetails((value) => !value)}
              type="button"
            >
              Technical details
              <ChevronDown
                className={`h-4 w-4 transition-transform ${showTechnicalDetails ? "rotate-180" : ""}`}
              />
            </button>
            {showTechnicalDetails ? (
              <div className="space-y-4 rounded-xl border border-border/60 bg-muted/15 p-4">
                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="connection-args"
                  >
                    Arguments
                  </label>
                  <Textarea
                    aria-describedby={
                      argsError ? "connection-args-error" : undefined
                    }
                    aria-invalid={Boolean(argsError) || undefined}
                    className="min-h-24 font-mono text-xs"
                    disabled={pending}
                    id="connection-args"
                    onChange={(event) => {
                      setArgsText(event.target.value);
                      setExecutionDirty(true);
                      setTrusted(false);
                    }}
                    placeholder={"--account\n123456"}
                    value={argsText}
                  />
                  <p className="text-xs text-muted-foreground">
                    Enter one argument per non-empty line. Buzz trims
                    surrounding whitespace. Do not put secrets here.
                  </p>
                  {argsError ? (
                    <p
                      className="text-xs text-destructive"
                      id="connection-args-error"
                    >
                      {argsError}
                    </p>
                  ) : null}
                </div>
                <div className="space-y-2">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="text-sm font-medium text-foreground">
                        Secrets
                      </p>
                      <p className="text-xs text-muted-foreground">
                        Values are saved locally and never shown again.
                      </p>
                    </div>
                    <Button
                      disabled={pending}
                      onClick={() => {
                        setSecrets((rows) => [...rows, emptySecretRow()]);
                        setExecutionDirty(true);
                        setTrusted(false);
                      }}
                      size="xs"
                      type="button"
                      variant="outline"
                    >
                      <Plus className="h-3.5 w-3.5" />
                      Add secret
                    </Button>
                  </div>
                  {secrets.length > 0 ? (
                    <div
                      aria-hidden="true"
                      className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] gap-2 px-0.5 text-2xs font-medium text-muted-foreground"
                    >
                      <span>Name</span>
                      <span>Value</span>
                      <span className="w-6" />
                    </div>
                  ) : null}
                  {secrets.map((row, index) => (
                    <div
                      className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)_auto] gap-2"
                      key={row.id}
                    >
                      <Input
                        aria-label={`Secret ${index + 1} name`}
                        autoCapitalize="characters"
                        autoCorrect="off"
                        disabled={
                          pending ||
                          Boolean(connection?.envKeys.includes(row.key))
                        }
                        onChange={(event) => {
                          const key = event.target.value.toUpperCase();
                          setSecrets((rows) =>
                            rows.map((item, rowIndex) =>
                              rowIndex === index ? { ...item, key } : item,
                            ),
                          );
                          setExecutionDirty(true);
                          setTrusted(false);
                        }}
                        placeholder="API_TOKEN"
                        spellCheck={false}
                        value={row.key}
                      />
                      <Input
                        aria-label={`Secret ${index + 1} value`}
                        autoComplete="off"
                        disabled={pending}
                        onChange={(event) => {
                          const value = event.target.value;
                          setSecrets((rows) =>
                            rows.map((item, rowIndex) =>
                              rowIndex === index ? { ...item, value } : item,
                            ),
                          );
                          setExecutionDirty(true);
                          setTrusted(false);
                        }}
                        placeholder={
                          connection?.envKeys.includes(row.key)
                            ? "Leave blank to keep saved value"
                            : "Secret value"
                        }
                        type="password"
                        value={row.value}
                      />
                      <Button
                        aria-label={`Remove secret ${index + 1}`}
                        disabled={pending}
                        onClick={() => {
                          if (connection?.envKeys.includes(row.key)) {
                            setRemovedEnvKeys((keys) => [...keys, row.key]);
                          }
                          setSecrets((rows) =>
                            rows.filter((_, rowIndex) => rowIndex !== index),
                          );
                          setExecutionDirty(true);
                          setTrusted(false);
                        }}
                        size="icon-xs"
                        type="button"
                        variant="ghost"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  ))}
                  {!secretChanges.ok ? (
                    <p className="text-xs text-destructive" role="alert">
                      {secretChanges.error}
                    </p>
                  ) : null}
                </div>
              </div>
            ) : null}
            {requiresApproval ? (
              <label
                className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-foreground"
                htmlFor="connection-trusted-command"
              >
                <Checkbox
                  checked={trusted}
                  disabled={pending}
                  id="connection-trusted-command"
                  onCheckedChange={(checked) => setTrusted(checked === true)}
                />
                <span>
                  I trust this executable and the arguments above to run without
                  a sandbox. It can access my files and network, plus these
                  secrets and anything their credentials allow.
                </span>
              </label>
            ) : null}
            {error ? (
              <p className="text-sm text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>
          <DialogFooter>
            <Button
              disabled={pending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button disabled={!canSubmit} type="submit">
              {pending
                ? "Saving…"
                : requiresApproval
                  ? "Save and test"
                  : "Save changes"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ConnectionRow({
  connection,
  onEdit,
  onRemove,
  onTest,
  testPending,
  testing,
}: {
  connection: ProjectConnection;
  onEdit: () => void;
  onRemove: () => void;
  onTest: () => void;
  testPending: boolean;
  testing: boolean;
}) {
  const recoveryNeedsEdit = connectionNeedsEditing(connection);
  const [showAllTools, setShowAllTools] = React.useState(false);
  const visibleTools = showAllTools
    ? connection.discoveredTools
    : connection.discoveredTools.slice(0, TOOL_PREVIEW_LIMIT);
  const hiddenToolCount = Math.max(
    0,
    connection.discoveredTools.length - TOOL_PREVIEW_LIMIT,
  );
  const actionLabel = connectionActionLabel(connection);

  return (
    <div
      aria-busy={testing || undefined}
      className="flex min-w-0 flex-wrap items-start gap-3 px-4 py-3"
      data-testid={`project-connection-${connection.id}`}
    >
      <Wrench className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1 basis-64 space-y-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <p
            className="min-w-0 truncate text-sm font-medium text-foreground"
            title={connection.name}
          >
            {connection.name}
          </p>
          <ConnectionHealthBadge status={connection.health.status} />
        </div>
        <div className="space-y-0.5">
          <p
            className="truncate text-xs text-muted-foreground"
            title={connection.provider}
          >
            {connection.provider}
          </p>
          <p
            aria-live="polite"
            className="text-xs text-muted-foreground"
            role="status"
          >
            {testing
              ? `Testing ${connection.name}…`
              : formatVerificationTime(connection.health.lastVerifiedAt)}
          </p>
          {connection.health.detail ? (
            <p className="text-xs text-muted-foreground">
              {connection.health.detail}
            </p>
          ) : null}
        </div>
        {visibleTools.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {visibleTools.map((tool) => (
              <span
                className="max-w-48 truncate rounded-md bg-muted px-2 py-1 text-2xs text-muted-foreground"
                key={tool}
                title={tool}
              >
                {toolLabel(tool)}
              </span>
            ))}
            {hiddenToolCount > 0 ? (
              <Button
                aria-expanded={showAllTools}
                className="h-6 px-1.5 text-2xs"
                onClick={() => setShowAllTools((value) => !value)}
                size="xs"
                type="button"
                variant="ghost"
              >
                {showAllTools ? "Show fewer" : `Show ${hiddenToolCount} more`}
              </Button>
            ) : null}
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">
            {connection.health.status === "not_tested"
              ? "Test this connection to discover its tools."
              : "No tools are currently available."}
          </p>
        )}
      </div>
      <div className="ml-7 flex shrink-0 items-center gap-1 sm:ml-0">
        <Button
          aria-label={
            testing
              ? `Testing ${connection.name}`
              : `${actionLabel} ${connection.name}`
          }
          disabled={testPending}
          onClick={recoveryNeedsEdit ? onEdit : onTest}
          size="sm"
          variant="outline"
        >
          {testing ? (
            <LoaderCircle className="h-4 w-4 animate-spin" />
          ) : recoveryNeedsEdit ? (
            <KeyRound className="h-4 w-4" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
          {testing ? "Testing…" : actionLabel}
        </Button>
        <Button
          aria-label={`Edit ${connection.name}`}
          disabled={testing}
          onClick={onEdit}
          size="icon-xs"
          title={`Edit ${connection.name}`}
          variant="ghost"
        >
          <Pencil className="h-4 w-4" />
        </Button>
        <Button
          aria-label={`Remove ${connection.name}`}
          disabled={testing}
          onClick={onRemove}
          size="icon-xs"
          title={`Remove ${connection.name}`}
          variant="ghost"
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

export function ProjectConnectionsPanel({
  projectScope,
}: {
  projectScope: ProjectConnectionScope;
}) {
  const query = useProjectConnectionsQuery(projectScope);
  const createMutation = useCreateProjectConnectionMutation(projectScope);
  const updateMutation = useUpdateProjectConnectionMutation(projectScope);
  const testMutation = useTestProjectConnectionMutation(projectScope);
  const deleteMutation = useDeleteProjectConnectionMutation(projectScope);
  const [dialogOpen, setDialogOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<ProjectConnection | null>(null);
  const [removing, setRemoving] = React.useState<ProjectConnection | null>(
    null,
  );
  const [removeError, setRemoveError] = React.useState<string | null>(null);

  function openAdd() {
    setEditing(null);
    setDialogOpen(true);
  }

  async function handleTest(connection: ProjectConnection) {
    try {
      const tested = await testMutation.mutateAsync(connection.id);
      if (tested.health.status === "ready") {
        toast.success(`Tools found for ${connection.name}.`);
      } else {
        toast.error(
          tested.health.detail ?? `${connection.name} needs attention.`,
        );
      }
    } catch (cause) {
      toast.error(
        cause instanceof Error
          ? `Couldn't test ${connection.name}: ${cause.message}`
          : `Couldn't test ${connection.name}. Check its details and try again.`,
      );
    }
  }

  async function handleDelete() {
    if (!removing) return;
    setRemoveError(null);
    try {
      await deleteMutation.mutateAsync(removing.id);
      toast.success(`${removing.name} removed.`);
      setRemoving(null);
    } catch (cause) {
      setRemoveError(
        cause instanceof Error
          ? `Couldn't remove ${removing.name}: ${cause.message}`
          : `Couldn't remove ${removing.name}. Nothing was changed.`,
      );
    }
  }

  const connections = query.data ?? [];

  return (
    <>
      <div
        className={PROJECT_DETAIL_PANEL_CLASS}
        data-project-detail-panel
        data-testid="project-connections-panel"
      >
        <div className="flex min-h-14 flex-wrap items-center gap-3 border-border/50 border-b px-4 py-3">
          <div className="flex min-w-[min(100%,18rem)] flex-1 items-start gap-3">
            <Link2 className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <h3 className="text-sm font-medium text-foreground">
                Connections
              </h3>
              <p className="text-xs text-muted-foreground">
                Connect MCP servers across this Project. Credentials stay on
                this device.
              </p>
            </div>
          </div>
          <Button
            className={`${PROJECT_PANEL_ACTION_BUTTON_CLASS} ml-auto`}
            onClick={openAdd}
            size="sm"
            title="Add connection"
          >
            <Plus className="h-4 w-4" />
            Add connection
          </Button>
        </div>

        {query.isPending ? (
          <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS} role="status">
            <LoaderCircle className="mx-auto mb-2 h-5 w-5 animate-spin" />
            Loading connections…
          </div>
        ) : query.isError ? (
          <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}>
            <XCircle className="mx-auto mb-2 h-5 w-5 text-destructive" />
            <p>
              Couldn't load connections. Your saved connections were not
              changed.
            </p>
            <Button
              className="mt-3"
              onClick={() => void query.refetch()}
              size="sm"
              variant="outline"
            >
              Try again
            </Button>
          </div>
        ) : connections.length === 0 ? (
          <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}>
            <Link2 className="mx-auto mb-2 h-5 w-5 text-muted-foreground" />
            <p className="font-medium text-foreground">No connections yet</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Add an MCP server, inspect its tools, and keep its credentials
              local to this device and associated with this Project.
            </p>
            <Button className="mt-3" onClick={openAdd} size="sm">
              Add connection
            </Button>
          </div>
        ) : (
          <div className="divide-y divide-border/50">
            {connections.map((connection) => (
              <ConnectionRow
                connection={connection}
                key={connection.id}
                onEdit={() => {
                  setEditing(connection);
                  setDialogOpen(true);
                }}
                onRemove={() => {
                  setRemoveError(null);
                  setRemoving(connection);
                }}
                onTest={() => void handleTest(connection)}
                testPending={testMutation.isPending}
                testing={
                  testMutation.isPending &&
                  testMutation.variables === connection.id
                }
              />
            ))}
          </div>
        )}
      </div>

      <ConnectionDialog
        connection={editing}
        onOpenChange={setDialogOpen}
        onSave={(input) =>
          input.id
            ? updateMutation.mutateAsync({
                ...input,
                id: input.id,
              })
            : createMutation.mutateAsync(input)
        }
        onTest={handleTest}
        open={dialogOpen}
        pending={createMutation.isPending || updateMutation.isPending}
        projectScope={projectScope}
      />

      <AlertDialog
        onOpenChange={(open) => {
          if (!open && !deleteMutation.isPending) {
            setRemoving(null);
            setRemoveError(null);
          }
        }}
        open={Boolean(removing)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {`Remove ${removing?.name ?? "connection"}?`}
            </AlertDialogTitle>
            <AlertDialogDescription>
              Buzz will remove the saved connection and its credentials from
              this device. This does not delete data from the connected service.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {removeError ? (
            <p className="text-sm text-destructive" role="alert">
              {removeError}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={deleteMutation.isPending}>
              Cancel
            </AlertDialogCancel>
            <Button
              disabled={deleteMutation.isPending}
              onClick={() => void handleDelete()}
              variant="destructive"
            >
              {deleteMutation.isPending ? "Removing…" : "Remove connection"}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
