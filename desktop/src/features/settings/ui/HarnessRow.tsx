import * as React from "react";
import { EllipsisVertical, ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpAuthMethodsQuery,
  useConnectAcpRuntimeMutation,
  useDeleteCustomHarnessMutation,
  useInstallAcpRuntimeMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
} from "@/features/agents/hooks";
import { RuntimeIcon } from "@/features/onboarding/ui/RuntimeIcon";
import type { AcpAuthMethod, AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Spinner } from "@/shared/ui/spinner";

import { CustomHarnessForm } from "./CustomHarnessForm";
import {
  adapterUpdateWarning,
  entryStatusLabel,
  isDownloadPageUrl,
} from "./harnessCatalogLogic";
import { formValuesFromCatalogEntry } from "./harnessFormLogic";
import { deleteConfirmState } from "./harnessGalleryLogic";

/** Link label for the row's install-instructions URL. Distinct from the
 * catalog's `installLinkLabel` — rows spell out what the guide covers
 * (adapter vs CLI) because the row lacks the catalog's setup context. */
function runtimeInstallGuideLabel(runtime: AcpRuntimeCatalogEntry) {
  if (
    runtime.availability === "adapter_missing" ||
    runtime.availability === "adapter_outdated"
  ) {
    return "Adapter install guide";
  }
  return isDownloadPageUrl(runtime.installInstructionsUrl)
    ? "Download page"
    : "CLI setup guide";
}

function RuntimeLogo({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  // Single logo pipeline: RuntimeIcon owns every runtime asset — the
  // theme-adaptive RUNTIME_MARKS, the BuzzMark, the bundled bitmap maps
  // (RUNTIME_LOGOS / PRESET_LOGOS), and the terminal-glyph fallback. It never
  // renders remote or user-supplied avatar URLs (security line), so the row
  // and the catalog cannot drift apart.
  return (
    <span
      className="flex h-9 w-9 shrink-0 items-center justify-center"
      data-testid={`doctor-runtime-logo-${runtime.id}`}
    >
      <RuntimeIcon className="h-9 w-9" runtime={runtime} />
    </span>
  );
}

function RuntimeOverflowMenu({
  authMethods,
  connectingMethodId,
  isConnecting,
  onConnect,
  onDelete,
  onEdit,
  runtime,
}: {
  authMethods: AcpAuthMethod[];
  connectingMethodId: string | null;
  isConnecting: boolean;
  onConnect: (method: AcpAuthMethod) => void;
  onDelete?: () => void;
  onEdit?: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const hasInstructions =
    runtime.installInstructionsUrl.trim().length > 0 &&
    (runtime.availability !== "available" ||
      runtime.authStatus.status === "logged_out" ||
      runtime.authStatus.status === "config_invalid");
  const hasActions =
    runtime.nodeRequired ||
    hasInstructions ||
    authMethods.length > 0 ||
    Boolean(onEdit) ||
    Boolean(onDelete);

  if (!hasActions) {
    return null;
  }

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Open actions for ${runtime.label}`}
          className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          data-testid={`doctor-runtime-menu-${runtime.id}`}
          type="button"
        >
          <EllipsisVertical className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        {authMethods.map((method) => (
          <DropdownMenuItem
            disabled={isConnecting}
            key={method.id}
            onSelect={() => onConnect(method)}
          >
            {isConnecting && connectingMethodId === method.id ? (
              <Spinner aria-hidden className="h-4 w-4 border-2" />
            ) : null}
            {method.name || method.id}
          </DropdownMenuItem>
        ))}
        {runtime.nodeRequired ? (
          <DropdownMenuItem onSelect={() => void openUrl("https://nodejs.org")}>
            <ExternalLink className="h-4 w-4" />
            Install Node.js
          </DropdownMenuItem>
        ) : null}
        {hasInstructions ? (
          <DropdownMenuItem
            onSelect={() => void openUrl(runtime.installInstructionsUrl)}
          >
            <ExternalLink className="h-4 w-4" />
            {runtimeInstallGuideLabel(runtime)}
          </DropdownMenuItem>
        ) : null}
        {onEdit ? (
          <DropdownMenuItem
            data-testid={`custom-harness-edit-${runtime.id}`}
            onSelect={onEdit}
          >
            Edit
          </DropdownMenuItem>
        ) : null}
        {onDelete ? (
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            data-testid={`custom-harness-delete-${runtime.id}`}
            onSelect={onDelete}
          >
            Delete
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function RuntimeActions({
  authMethods,
  connectingMethodId,
  isConnecting,
  isInstalling,
  onConnect,
  onDelete,
  onEdit,
  onInstall,
  runtime,
}: {
  authMethods: AcpAuthMethod[];
  connectingMethodId: string | null;
  isConnecting: boolean;
  isInstalling: boolean;
  onConnect: (method: AcpAuthMethod) => void;
  onDelete?: () => void;
  onEdit?: () => void;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const isAvailable = runtime.availability === "available";
  // Signed-out rows carry the amber "Sign-in needed" status chip instead of a
  // green Ready chip — auth-required is an explicit row-face state, and Ready
  // must not claim otherwise.
  const isAuthNeeded =
    isAvailable && runtime.authStatus.status === "logged_out";
  const canInstall = runtime.canAutoInstall && !runtime.nodeRequired;
  const isWorking = isInstalling || isConnecting;

  return (
    <div className="ml-auto flex shrink-0 items-center justify-end gap-1">
      <RuntimeOverflowMenu
        authMethods={authMethods}
        connectingMethodId={connectingMethodId}
        isConnecting={isConnecting}
        onConnect={onConnect}
        onDelete={onDelete}
        onEdit={onEdit}
        runtime={runtime}
      />
      {isWorking ? (
        <div className="flex h-7 w-9 items-center justify-center text-muted-foreground">
          <Spinner
            aria-label={`${runtime.label} ${isInstalling ? "installing" : "connecting"}`}
            className="h-4 w-4 border-2"
            data-testid={`doctor-runtime-loading-${runtime.id}`}
          />
        </div>
      ) : isAvailable ? (
        isAuthNeeded ? null : ( // Signed-out rows carry the amber status chip instead; never Install.
          <span
            className="inline-flex shrink-0 items-center rounded-md bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400"
            data-testid={`doctor-runtime-ready-${runtime.id}`}
          >
            Ready
          </span>
        )
      ) : canInstall ? (
        // Rows needing multi-step setup render no action here — setup lives in
        // the Add-runtimes catalog. Custom rows keep their ••• menu instead.
        <Button
          aria-label={`Install ${runtime.label}`}
          className="h-7 px-3 text-xs"
          data-testid={`doctor-runtime-install-${runtime.id}`}
          onClick={onInstall}
          size="sm"
          type="button"
          variant="outline"
        >
          {runtime.availability === "adapter_outdated" ? "Update" : "Install"}
        </Button>
      ) : null}
    </div>
  );
}

function RuntimeStatusChip({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  // Single availability→label source: entryStatusLabel drives this row chip
  // AND the catalog detail chip, so the two surfaces cannot drift. That
  // includes "Sign-in needed" for installed-but-signed-out runtimes — an
  // explicit auth-required state on the row face, not just a ••• menu item.
  const label = entryStatusLabel(runtime);

  if (!label) {
    return null;
  }

  const isConfigError = runtime.authStatus.status === "config_invalid";
  const isAuthNeeded =
    !isConfigError &&
    runtime.availability === "available" &&
    runtime.authStatus.status === "logged_out";

  return (
    <>
      <span aria-hidden="true" className="text-muted-foreground/50">
        ·
      </span>
      <span
        className={cn(
          "inline-flex shrink-0 items-center rounded-md px-2 py-0.5 text-xs font-medium",
          isConfigError
            ? "bg-destructive/10 text-destructive"
            : isAuthNeeded
              ? "bg-amber-500/15 text-amber-600 dark:text-amber-400"
              : "bg-muted text-muted-foreground",
        )}
        data-testid={`doctor-runtime-status-${runtime.id}`}
      >
        {label}
      </span>
    </>
  );
}

/**
 * One row in "Your runtimes".
 *
 * Carries the full operational surface for a ready (or one-click-ready)
 * harness: logo, status chip, auth/overflow menu, install/connect flows, and
 * — for custom harnesses — edit and delete with the blast-radius guard.
 */
export function HarnessRow({
  resetEpoch,
  runtime,
}: {
  resetEpoch: number;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const isCustom = runtime.source === "custom";
  const [terminalLaunchMethodId, setTerminalLaunchMethodId] = React.useState<
    string | null
  >(null);
  const [isUpdateWarningOpen, setIsUpdateWarningOpen] = React.useState(false);
  const [editing, setEditing] = React.useState(false);
  const [confirmingDelete, setConfirmingDelete] = React.useState(false);
  const [deleteError, setDeleteError] = React.useState<string | null>(null);
  // Each row owns its mutation instance so concurrent installs each track
  // their own isPending / result state independently.
  const installMutation = useInstallAcpRuntimeMutation();
  const [installResult, setInstallResult] = React.useState<{
    success: boolean;
    error: string | null;
  } | null>(null);
  // Clear stale install results when the parent triggers a catalog refresh
  // (Check again) — the runtime may now be healthy and stale failure state
  // would linger because keyed rows don't remount on refetch.
  // biome-ignore lint/correctness/useExhaustiveDependencies: resetEpoch is an intentional trigger only; its value is not consumed in the effect body
  React.useEffect(() => {
    setInstallResult(null);
  }, [resetEpoch]);
  const isInstalling = installMutation.isPending;
  const installError = installResult?.error ?? null;

  const del = useDeleteCustomHarnessMutation();
  // Blast-radius data for the delete confirmation — only fetched while the
  // confirmation is open, so the row list doesn't poll agents. Confirm stays
  // disabled until both queries settle (deleteConfirmState) so a quick click
  // can't beat the "N agents will stop launching" warning.
  const agentsQuery = useManagedAgentsQuery({ enabled: confirmingDelete });
  const personasQuery = usePersonasQuery({ enabled: confirmingDelete });
  const confirmState = deleteConfirmState(
    runtime.id,
    runtime.label,
    agentsQuery,
    personasQuery,
  );

  function handleInstall() {
    setInstallResult(null);
    installMutation.mutate(runtime.id, {
      onSuccess: (result) => {
        if (result.success) {
          setInstallResult({ success: true, error: null });
        } else {
          setInstallResult({
            success: false,
            error: getInstallErrorMessage(result.steps),
          });
        }
      },
      onError: (error) => {
        setInstallResult({
          success: false,
          error: error instanceof Error ? error.message : "Install failed.",
        });
      },
    });
  }

  const canConnectAccount =
    runtime.availability === "available" &&
    runtime.authStatus.status === "logged_out";
  const authMethodsQuery = useAcpAuthMethodsQuery(runtime.id, {
    enabled: canConnectAccount,
  });
  const authMethods = canConnectAccount
    ? (authMethodsQuery.data?.methods ?? [])
    : [];
  const connectMutation = useConnectAcpRuntimeMutation();
  const connectionError = connectMutation.error
    ? `Couldn't connect ${runtime.label}: ${
        connectMutation.error instanceof Error
          ? connectMutation.error.message
          : "Connection failed."
      }`
    : authMethodsQuery.error
      ? `Couldn't load sign-in options: ${
          authMethodsQuery.error instanceof Error
            ? authMethodsQuery.error.message
            : "Request failed."
        }`
      : null;

  if (editing) {
    return (
      <CustomHarnessForm
        initial={formValuesFromCatalogEntry(runtime)}
        originalId={runtime.id}
        onCancel={() => setEditing(false)}
        onSaved={() => setEditing(false)}
      />
    );
  }

  return (
    <div
      className="min-h-16 rounded-2xl border border-border/60 bg-muted/20 px-4 py-3.5 text-sm"
      data-testid={`doctor-runtime-${runtime.id}`}
    >
      <div className="min-w-0">
        <div className="flex items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <RuntimeLogo runtime={runtime} />
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <p className="min-w-0 text-sm font-medium">{runtime.label}</p>
              <RuntimeStatusChip runtime={runtime} />
            </div>
          </div>
          <RuntimeActions
            authMethods={authMethods}
            connectingMethodId={connectMutation.variables?.methodId ?? null}
            isConnecting={connectMutation.isPending}
            isInstalling={isInstalling}
            onConnect={(method) => {
              setTerminalLaunchMethodId(null);
              connectMutation.mutate(
                {
                  runtimeId: runtime.id,
                  methodId: method.id,
                },
                {
                  onSuccess: (result) => {
                    if (result.launched && method.type === "terminal") {
                      setTerminalLaunchMethodId(method.id);
                    }
                  },
                },
              );
            }}
            onDelete={
              isCustom
                ? () => {
                    setDeleteError(null);
                    setConfirmingDelete(true);
                  }
                : undefined
            }
            onEdit={isCustom ? () => setEditing(true) : undefined}
            onInstall={() => {
              if (runtime.availability === "adapter_outdated") {
                setIsUpdateWarningOpen(true);
                return;
              }
              handleInstall();
            }}
            runtime={runtime}
          />
        </div>

        {runtime.availability !== "available" ? (
          <div
            className="mt-2 flex flex-wrap items-center justify-between gap-2 text-sm text-muted-foreground"
            data-testid={`doctor-runtime-guidance-${runtime.id}`}
          >
            <p>{runtime.installHint}</p>
            {runtime.installInstructionsUrl.trim().length > 0 ? (
              <button
                className="inline-flex shrink-0 items-center gap-1 underline-offset-2 hover:text-foreground hover:underline"
                onClick={() => void openUrl(runtime.installInstructionsUrl)}
                type="button"
              >
                <ExternalLink className="h-4 w-4" />
                {runtimeInstallGuideLabel(runtime)}
              </button>
            ) : null}
          </div>
        ) : null}

        {runtime.authStatus.status === "config_invalid" ? (
          <p
            className="mt-2 whitespace-pre-line rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive"
            data-testid={`doctor-runtime-config-error-${runtime.id}`}
          >
            Config error: {runtime.authStatus.diagnostic}
          </p>
        ) : null}

        {installError ? (
          <p
            className="mt-2 whitespace-pre-line rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive"
            data-testid={`doctor-runtime-install-error-${runtime.id}`}
          >
            {installError}
          </p>
        ) : null}
        {connectionError ? (
          <p
            className="mt-2 whitespace-pre-line rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive"
            data-testid={`doctor-runtime-error-${runtime.id}`}
          >
            {connectionError}
          </p>
        ) : null}
        {canConnectAccount && terminalLaunchMethodId ? (
          <p
            className="mt-2 rounded-lg border border-border/60 bg-background/60 px-3 py-1.5 text-sm text-muted-foreground"
            data-testid={`doctor-runtime-terminal-guidance-${runtime.id}`}
          >
            Finish signing in from the Terminal window, then click Check again
            to re-check {runtime.label}.
          </p>
        ) : null}
        {confirmingDelete ? (
          <div className="mt-2 space-y-2">
            <p
              className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-sm text-amber-600 dark:text-amber-400"
              data-testid={`custom-harness-delete-warning-${runtime.id}`}
            >
              {confirmState.message}
            </p>
            <div className="flex justify-end gap-2">
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
              <Button
                className="h-7 px-3 text-xs"
                data-testid={`custom-harness-delete-confirm-${runtime.id}`}
                disabled={del.isPending || !confirmState.canConfirm}
                onClick={() => {
                  setDeleteError(null);
                  del.mutate(runtime.id, {
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
            </div>
          </div>
        ) : null}
        {deleteError ? (
          <p className="mt-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
            {deleteError}
          </p>
        ) : null}
      </div>
      <AlertDialog
        onOpenChange={setIsUpdateWarningOpen}
        open={isUpdateWarningOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Update {runtime.label} adapter?</AlertDialogTitle>
            <AlertDialogDescription>
              {adapterUpdateWarning(runtime)}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleInstall}
              data-testid={`doctor-runtime-confirm-update-${runtime.id}`}
            >
              Update
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
