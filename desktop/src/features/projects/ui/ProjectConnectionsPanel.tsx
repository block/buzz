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
  AlertDialogAction,
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

function emptySecretRow(): SecretRow {
  return { id: crypto.randomUUID(), key: "", value: "" };
}

function capabilityLabel(capability: string) {
  return capability
    .replace(/^mcp\.tool\./, "")
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
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
  open,
  pending,
  projectScope,
}: {
  connection: ProjectConnection | null;
  onOpenChange: (open: boolean) => void;
  onSave: (input: ProjectConnectionDraft & { id?: string }) => Promise<unknown>;
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
    setTrusted(Boolean(connection));
    setError(null);
  }, [connection, open]);

  const canSave =
    !pending &&
    name.trim().length > 0 &&
    provider.trim().length > 0 &&
    command.trim().length > 0;
  const canSubmit = canSave && trusted;

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    if (!canSubmit) return;
    const secretChanges = buildProjectConnectionSecretChanges(
      secrets,
      connection?.envKeys ?? [],
      removedEnvKeys,
    );
    if (!secretChanges.ok) {
      setError(secretChanges.error);
      return;
    }
    setError(null);
    try {
      await onSave({
        ...(connection ? { id: connection.id } : {}),
        projectScope,
        name: name.trim(),
        provider: provider.trim(),
        command: command.trim(),
        args: argsText
          .split("\n")
          .map((arg) => arg.trim())
          .filter(Boolean),
        env: secretChanges.env,
        removeEnvKeys: secretChanges.removeEnvKeys,
        executionAcknowledged: trusted,
      });
      onOpenChange(false);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Couldn't save this connection. Check the details and try again.",
      );
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>
              {connection ? "Edit connection" : "Add connection"}
            </DialogTitle>
            <DialogDescription>
              {connection
                ? "Save changes to this Project connection. Test it again after changing how it runs."
                : "Save a local MCP connection for this Project. Secret values stay outside portable agent configuration."}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-5">
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="connection-name"
              >
                Connection name
              </label>
              <Input
                disabled={pending}
                id="connection-name"
                onChange={(event) => setName(event.target.value)}
                placeholder="Analytics"
                value={name}
              />
            </div>
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="connection-provider"
              >
                Service
              </label>
              <Input
                disabled={pending}
                id="connection-provider"
                onChange={(event) => setProvider(event.target.value)}
                placeholder="Google Analytics"
                value={provider}
              />
            </div>
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="connection-command"
              >
                Connection command
              </label>
              <Input
                autoCapitalize="off"
                autoCorrect="off"
                disabled={pending}
                id="connection-command"
                onChange={(event) => {
                  setCommand(event.target.value);
                  setTrusted(false);
                }}
                placeholder="/absolute/path/to/mcp-server"
                spellCheck={false}
                value={command}
              />
              <p className="text-xs text-muted-foreground">
                Enter the executable's absolute path. Buzz runs it directly
                without a shell.
              </p>
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
                    className="min-h-24 font-mono text-xs"
                    disabled={pending}
                    id="connection-args"
                    onChange={(event) => {
                      setArgsText(event.target.value);
                      setTrusted(false);
                    }}
                    placeholder={"--account\n123456"}
                    value={argsText}
                  />
                  <p className="text-xs text-muted-foreground">
                    Enter one argument per non-empty line. Buzz trims
                    surrounding whitespace. Do not put secrets here.
                  </p>
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
                </div>
              </div>
            ) : null}
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
                I trust this executable and the arguments above to run without a
                sandbox. It can access my files and network, plus these secrets
                and anything their credentials allow.
              </span>
            </label>
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
              {pending ? "Saving..." : "Save connection"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
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

  function openAdd() {
    setEditing(null);
    setDialogOpen(true);
  }

  async function handleDelete() {
    if (!removing) return;
    try {
      await deleteMutation.mutateAsync(removing.id);
      toast.success(`${removing.name} removed.`);
      setRemoving(null);
    } catch (cause) {
      toast.error(
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
        <div className="flex min-h-14 items-center gap-3 border-border/50 border-b px-4 py-3">
          <Link2 className="h-4 w-4 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-medium text-foreground">Connections</h3>
            <p className="text-xs text-muted-foreground">
              Save and verify MCP connections for this Project.
            </p>
          </div>
          <Button onClick={openAdd} size="sm" title="Add connection">
            <Plus className="h-4 w-4" />
            Add connection
          </Button>
        </div>

        {query.isPending ? (
          <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}>
            <LoaderCircle className="mx-auto mb-2 h-5 w-5 animate-spin" />
            Loading connections...
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
              <div
                className="flex items-start gap-3 px-4 py-4"
                key={connection.id}
              >
                <div className="mt-0.5 rounded-lg bg-muted p-2">
                  <KeyRound className="h-4 w-4 text-muted-foreground" />
                </div>
                <div className="min-w-0 flex-1 space-y-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="font-medium text-foreground">
                      {connection.name}
                    </p>
                    <ConnectionHealthBadge status={connection.health.status} />
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {connection.provider} ·{" "}
                    {formatVerificationTime(connection.health.lastVerifiedAt)}
                  </p>
                  {connection.health.detail ? (
                    <p className="text-xs text-muted-foreground">
                      {connection.health.detail}
                    </p>
                  ) : null}
                  {connection.capabilityIds.length > 0 ? (
                    <div className="flex flex-wrap gap-1.5">
                      {connection.capabilityIds.map((capability) => (
                        <span
                          className="inline-flex items-center gap-1 rounded-md border border-border/60 bg-muted/30 px-2 py-1 text-2xs text-muted-foreground"
                          key={capability}
                          title={capability}
                        >
                          <Wrench className="h-3 w-3" />
                          {capabilityLabel(capability)}
                        </span>
                      ))}
                    </div>
                  ) : (
                    <p className="text-xs text-muted-foreground">
                      Test this connection to discover its tools.
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <Button
                    disabled={testMutation.isPending}
                    onClick={async () => {
                      try {
                        const tested = await testMutation.mutateAsync(
                          connection.id,
                        );
                        if (tested.health.status === "ready") {
                          toast.success(`Tools found for ${connection.name}.`);
                        } else {
                          toast.error(
                            tested.health.detail ??
                              `${connection.name} needs attention.`,
                          );
                        }
                      } catch (cause) {
                        toast.error(
                          cause instanceof Error
                            ? `Couldn't test ${connection.name}: ${cause.message}`
                            : `Couldn't test ${connection.name}. Check its details and try again.`,
                        );
                      }
                    }}
                    size="sm"
                    variant="outline"
                  >
                    {testMutation.isPending ? (
                      <LoaderCircle className="h-4 w-4 animate-spin" />
                    ) : (
                      <RefreshCw className="h-4 w-4" />
                    )}
                    Test
                  </Button>
                  <Button
                    aria-label={`Edit ${connection.name}`}
                    onClick={() => {
                      setEditing(connection);
                      setDialogOpen(true);
                    }}
                    size="icon-xs"
                    variant="ghost"
                  >
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button
                    aria-label={`Remove ${connection.name}`}
                    onClick={() => setRemoving(connection)}
                    size="icon-xs"
                    variant="ghost"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </div>
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
        open={dialogOpen}
        pending={createMutation.isPending || updateMutation.isPending}
        projectScope={projectScope}
      />

      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setRemoving(null);
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
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                disabled={deleteMutation.isPending}
                onClick={() => void handleDelete()}
                variant="destructive"
              >
                {deleteMutation.isPending ? "Removing..." : "Remove connection"}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
