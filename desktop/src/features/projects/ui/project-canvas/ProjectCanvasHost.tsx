import { listen } from "@tauri-apps/api/event";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  AlertTriangle,
  FolderOpen,
  LoaderCircle,
  RefreshCw,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { ProjectCanvasBroker } from "./projectCanvasBroker";
import {
  openProjectCanvasSource,
  projectCanvasErrorMessage,
  releaseProjectCanvasPackage,
  requestProjectCanvasPackage,
  requestProjectCanvasUpdates,
} from "./projectCanvasCommands";
import {
  effectiveProjectCanvasCapabilities,
  readProjectCanvasConsent,
  writeProjectCanvasConsent,
  type ProjectCanvasConsentDecision,
} from "./projectCanvasConsent";
import { ProjectCanvasFrame } from "./ProjectCanvasFrame";
import {
  readProjectCanvasLayouts,
  writeProjectCanvasDashboardLayout,
} from "./projectCanvasLayout";
import {
  parseProjectCanvasSourceUpdateEvent,
  projectCanvasConsentCapabilities,
  type ProjectCanvasLayoutMessage,
  type ProjectCanvasLayouts,
  type ProjectCanvasPackageDescriptor,
  type ProjectCanvasPendingUpdates,
  type ProjectCanvasSnapshots,
} from "./projectCanvasProtocol";

type ProjectCanvasHostProps = {
  broker: ProjectCanvasBroker | null;
  communityId: string | null;
  full: boolean;
  projectName: string;
  projectNames: readonly string[];
  projectId: string;
  snapshots: ProjectCanvasSnapshots;
};

const CONSENT_CAPABILITY_LABELS: Record<string, string> = {
  "app.dm.send": "send direct messages as you",
  "app.open": "open channels, people, and work items",
  "project.tasks.write": "update project tasks",
};

const PROJECT_CANVAS_SOURCE_UPDATE_EVENT = "project-canvas-source-updated";

function commandToastLabel(commandName: string): string {
  if (commandName === "tasks.setStatus") return "updated a task's status";
  if (commandName === "tasks.assign") return "assigned a task";
  if (commandName === "tasks.unassign") return "unassigned a task";
  if (commandName === "dm.send") return "sent a direct message";
  return "ran a command";
}

function commandFailureTitle(commandName: string): string {
  return commandName === "dm.send"
    ? "Canvas direct message failed"
    : "Canvas task update failed";
}

function consentPhrase(labels: string[]): string {
  if (labels.length > 2) {
    return `${labels.slice(0, -1).join(", ")}, and ${labels.at(-1)}`;
  }
  return labels.join(" and ");
}

export function ProjectCanvasHost({
  broker,
  communityId,
  full,
  projectId,
  projectName,
  projectNames,
  snapshots,
}: ProjectCanvasHostProps) {
  const [descriptor, setDescriptor] =
    React.useState<ProjectCanvasPackageDescriptor | null>(null);
  const [consentVersion, setConsentVersion] = React.useState(0);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [reloading, setReloading] = React.useState(false);
  const [dataUpdate, setDataUpdate] =
    React.useState<ProjectCanvasPendingUpdates["data"]>(null);
  const requestGenerationRef = React.useRef(0);
  const candidateLoadIdRef = React.useRef<string | null>(null);
  const restoredLoadIdRef = React.useRef<string | null>(null);
  const lastDataNotificationRef = React.useRef<string | null>(null);
  const lastPresentationNotificationRef = React.useRef<string | null>(null);
  const bindingKey = `${communityId ?? ""}\u0000${projectId}`;
  const bindingKeyRef = React.useRef(bindingKey);
  bindingKeyRef.current = bindingKey;

  React.useEffect(
    () => () => {
      requestGenerationRef.current += 1;
    },
    [],
  );

  React.useEffect(() => {
    let disposed = false;
    const generation = ++requestGenerationRef.current;
    const requestedBinding = bindingKey;
    candidateLoadIdRef.current = null;
    restoredLoadIdRef.current = null;
    setDescriptor(null);
    setLoadError(null);
    setReloading(false);
    setDataUpdate(null);
    lastDataNotificationRef.current = null;
    lastPresentationNotificationRef.current = null;

    if (!communityId) {
      setLoadError("Canvas is unavailable until the community is ready.");
      return () => {
        disposed = true;
      };
    }

    void requestProjectCanvasPackage("get_project_canvas_package", {
      communityId,
      projectId,
    })
      .then((nextDescriptor) => {
        if (
          disposed ||
          requestGenerationRef.current !== generation ||
          bindingKeyRef.current !== requestedBinding
        ) {
          void releaseProjectCanvasPackage(nextDescriptor.loadId).catch(
            () => {},
          );
          return;
        }
        setDescriptor(nextDescriptor);
      })
      .catch((error: unknown) => {
        if (
          !disposed &&
          requestGenerationRef.current === generation &&
          bindingKeyRef.current === requestedBinding
        ) {
          setLoadError(projectCanvasErrorMessage(error));
        }
      });

    return () => {
      disposed = true;
    };
  }, [bindingKey, communityId, projectId]);

  React.useEffect(() => {
    if (!communityId) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    let queued = Promise.resolve();
    const requestedBinding = bindingKey;

    const sync = async () => {
      let updates: ProjectCanvasPendingUpdates;
      try {
        updates = await requestProjectCanvasUpdates({ communityId, projectId });
      } catch (error) {
        if (!disposed && bindingKeyRef.current === requestedBinding) {
          setLoadError(projectCanvasErrorMessage(error));
        }
        return;
      }
      if (disposed || bindingKeyRef.current !== requestedBinding) {
        if (updates.presentation) {
          await releaseProjectCanvasPackage(
            updates.presentation.package.loadId,
          ).catch(() => {});
        }
        return;
      }

      if (updates.presentation) {
        if (
          updates.presentation.notificationId ===
          lastPresentationNotificationRef.current
        ) {
          await releaseProjectCanvasPackage(
            updates.presentation.package.loadId,
          ).catch(() => {});
        } else {
          lastPresentationNotificationRef.current =
            updates.presentation.notificationId;
          requestGenerationRef.current += 1;
          candidateLoadIdRef.current = updates.presentation.package.loadId;
          setReloading(true);
          setLoadError(null);
          setDescriptor(updates.presentation.package);
        }
      }
      if (
        updates.data &&
        updates.data.notificationId !== lastDataNotificationRef.current
      ) {
        lastDataNotificationRef.current = updates.data.notificationId;
        setDataUpdate(updates.data);
      }
    };
    const scheduleSync = () => {
      queued = queued.then(sync, sync);
    };

    void listen<unknown>(PROJECT_CANVAS_SOURCE_UPDATE_EVENT, (event) => {
      const binding = parseProjectCanvasSourceUpdateEvent(event.payload);
      if (
        binding?.communityId === communityId &&
        binding.projectId === projectId
      ) {
        scheduleSync();
      }
    })
      .then((stop) => {
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
        scheduleSync();
      })
      .catch((error: unknown) => {
        if (!disposed && bindingKeyRef.current === requestedBinding) {
          setLoadError(projectCanvasErrorMessage(error));
        }
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [bindingKey, communityId, projectId]);

  React.useEffect(() => {
    if (!descriptor) return;
    return () => {
      void releaseProjectCanvasPackage(descriptor.loadId).catch(() => {});
    };
  }, [descriptor]);

  const reload = React.useCallback(async () => {
    if (!communityId || reloading) return;
    const generation = ++requestGenerationRef.current;
    const requestedBinding = bindingKey;
    setReloading(true);
    setLoadError(null);
    try {
      const nextDescriptor = await requestProjectCanvasPackage(
        "activate_project_canvas_package",
        { communityId, projectId },
      );
      if (
        requestGenerationRef.current !== generation ||
        bindingKeyRef.current !== requestedBinding
      ) {
        await releaseProjectCanvasPackage(nextDescriptor.loadId).catch(
          () => {},
        );
        return;
      }
      candidateLoadIdRef.current = nextDescriptor.loadId;
      setDescriptor(nextDescriptor);
    } catch (error) {
      if (
        requestGenerationRef.current === generation &&
        bindingKeyRef.current === requestedBinding
      ) {
        setLoadError(projectCanvasErrorMessage(error));
      }
    } finally {
      if (requestGenerationRef.current === generation) {
        setReloading(false);
      }
    }
  }, [bindingKey, communityId, projectId, reloading]);

  const handleFrameFailure = React.useCallback(
    (loadId: string, message: string) => {
      if (candidateLoadIdRef.current !== loadId || !communityId) {
        setDescriptor((current) =>
          current?.loadId === loadId ? null : current,
        );
        setLoadError(message);
        return;
      }

      candidateLoadIdRef.current = null;
      const generation = ++requestGenerationRef.current;
      const requestedBinding = bindingKey;
      setLoadError(
        `Canvas reload failed; restored the active version. ${message}`,
      );
      setReloading(true);
      void requestProjectCanvasPackage("get_project_canvas_package", {
        communityId,
        projectId,
      })
        .then((activeDescriptor) => {
          if (
            requestGenerationRef.current !== generation ||
            bindingKeyRef.current !== requestedBinding
          ) {
            void releaseProjectCanvasPackage(activeDescriptor.loadId).catch(
              () => {},
            );
            return;
          }
          restoredLoadIdRef.current = activeDescriptor.loadId;
          setDescriptor(activeDescriptor);
        })
        .catch((error: unknown) => {
          if (
            requestGenerationRef.current === generation &&
            bindingKeyRef.current === requestedBinding
          ) {
            setDescriptor(null);
            setLoadError(
              `Canvas reload failed and the active version could not be restored. ${projectCanvasErrorMessage(error)}`,
            );
          }
        })
        .finally(() => {
          if (requestGenerationRef.current === generation) {
            setReloading(false);
          }
        });
    },
    [bindingKey, communityId, projectId],
  );

  const handleFrameRendered = React.useCallback((loadId: string) => {
    if (restoredLoadIdRef.current === loadId) {
      restoredLoadIdRef.current = null;
      return;
    }
    if (candidateLoadIdRef.current === loadId) {
      candidateLoadIdRef.current = null;
    }
    setLoadError(null);
    setReloading(false);
  }, []);

  const openSource = React.useCallback(async () => {
    if (!communityId) return;
    try {
      await openProjectCanvasSource({ communityId, projectId });
    } catch (error) {
      setLoadError(projectCanvasErrorMessage(error));
    }
  }, [communityId, projectId]);

  const requestedConsentCapabilities = React.useMemo(
    () => projectCanvasConsentCapabilities(descriptor?.capabilities ?? []),
    [descriptor?.capabilities],
  );
  const consentDecision = React.useMemo(() => {
    void consentVersion;
    if (!communityId || !descriptor) return null;
    return readProjectCanvasConsent(
      communityId,
      projectId,
      descriptor.revision,
    );
  }, [communityId, consentVersion, descriptor, projectId]);
  const consentPending =
    requestedConsentCapabilities.length > 0 && consentDecision === null;
  const capabilities = React.useMemo(
    () =>
      effectiveProjectCanvasCapabilities(
        descriptor?.capabilities ?? [],
        consentDecision,
      ),
    [consentDecision, descriptor?.capabilities],
  );
  const decideConsent = React.useCallback(
    (decision: ProjectCanvasConsentDecision) => {
      if (!communityId || !descriptor) return;
      writeProjectCanvasConsent(
        communityId,
        projectId,
        descriptor.revision,
        decision,
      );
      setConsentVersion((version) => version + 1);
    },
    [communityId, descriptor, projectId],
  );
  // Both callbacks capture the binding they were built for, so a layout
  // message that lands while the host is switching community or project can
  // never read or write under the next binding's key.
  const loadLayouts = React.useCallback((): ProjectCanvasLayouts => {
    if (!communityId || bindingKeyRef.current !== bindingKey) return {};
    return readProjectCanvasLayouts(communityId, projectId);
  }, [bindingKey, communityId, projectId]);
  const handleLayoutChanged = React.useCallback(
    (message: ProjectCanvasLayoutMessage) => {
      if (!communityId || bindingKeyRef.current !== bindingKey) return;
      writeProjectCanvasDashboardLayout(
        communityId,
        projectId,
        message.dashboard,
        {
          pan: message.pan,
          sizes: message.sizes ?? {},
          widgets: message.widgets,
        },
      );
    },
    [bindingKey, communityId, projectId],
  );
  const handleCommandSettled = React.useCallback(
    (commandName: string, commandError: string | null) => {
      if (commandError) {
        toast.error(commandFailureTitle(commandName), {
          description: commandError,
        });
        return;
      }
      toast.success(`Canvas ${commandToastLabel(commandName)}`);
    },
    [],
  );

  return (
    <section
      aria-busy={!descriptor && !loadError}
      aria-label="Project widget canvas"
      className="relative h-full min-h-0 w-full overflow-hidden bg-muted/35"
      data-testid="project-widget-canvas"
    >
      {descriptor ? (
        <ProjectCanvasFrame
          broker={broker}
          capabilities={capabilities}
          descriptor={descriptor}
          dataUpdate={dataUpdate}
          key={`${descriptor.loadId}:${consentDecision ?? "pending"}`}
          loadLayouts={loadLayouts}
          mode={full ? "full" : "preview"}
          onCommandSettled={handleCommandSettled}
          onFailure={handleFrameFailure}
          onLayoutChanged={handleLayoutChanged}
          onRendered={handleFrameRendered}
          projectId={projectId}
          projectName={projectName}
          projectNames={projectNames}
          snapshots={snapshots}
        />
      ) : loadError ? (
        <CanvasFailure message={loadError} onReload={() => void reload()} />
      ) : (
        <div className="flex h-full items-center justify-center text-muted-foreground">
          <LoaderCircle
            aria-label="Loading Canvas"
            className="h-5 w-5 animate-spin"
          />
        </div>
      )}

      {communityId ? (
        <div className="absolute right-3 top-3 z-40 flex gap-1">
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <Button
                aria-label="Open Canvas files"
                className="h-8 w-8 border-border/80 bg-background/95 shadow-sm"
                data-testid="project-canvas-open-source"
                onClick={() => void openSource()}
                size="icon"
                type="button"
                variant="outline"
              >
                <FolderOpen className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Open Canvas files</TooltipContent>
          </Tooltip>
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <Button
                aria-label="Reload Canvas"
                className="h-8 w-8 border-border/80 bg-background/95 shadow-sm"
                data-testid="project-canvas-reload"
                disabled={reloading}
                onClick={() => void reload()}
                size="icon"
                type="button"
                variant="outline"
              >
                <RefreshCw
                  className={cn("h-4 w-4", reloading && "animate-spin")}
                />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Reload Canvas</TooltipContent>
          </Tooltip>
        </div>
      ) : null}
      {descriptor && loadError ? (
        <div className="absolute inset-x-14 top-3 z-40 flex justify-center">
          <div
            className="flex max-w-xl items-center gap-3 rounded-sm border border-destructive/35 bg-background/95 px-3 py-2 text-xs text-destructive shadow-sm"
            data-testid="project-canvas-reload-error"
            role="alert"
          >
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <span className="min-w-0 flex-1 break-words">{loadError}</span>
            <Button
              className="shrink-0"
              disabled={reloading}
              onClick={() => void reload()}
              size="sm"
              type="button"
              variant="outline"
            >
              <RefreshCw
                className={cn("h-4 w-4", reloading && "animate-spin")}
              />
              Retry
            </Button>
          </div>
        </div>
      ) : null}
      {descriptor && consentPending ? (
        <div className="absolute inset-x-14 bottom-3 z-40 flex justify-center">
          <section
            aria-label="Canvas permission request"
            className="flex max-w-xl flex-wrap items-center gap-3 rounded-sm border border-border/80 bg-background/95 px-3 py-2 text-xs shadow-sm"
            data-testid="project-canvas-consent"
          >
            <span className="min-w-0 flex-1 break-words text-muted-foreground">
              This Canvas asks to{" "}
              {consentPhrase(
                requestedConsentCapabilities.map(
                  (capability) =>
                    CONSENT_CAPABILITY_LABELS[capability] ?? capability,
                ),
              )}
              . Denying keeps the read-only canvas running.
            </span>
            <div className="flex shrink-0 gap-1">
              <Button
                data-testid="project-canvas-consent-deny"
                onClick={() => decideConsent("denied")}
                size="sm"
                type="button"
                variant="outline"
              >
                Don't allow
              </Button>
              <Button
                data-testid="project-canvas-consent-approve"
                onClick={() => decideConsent("approved")}
                size="sm"
                type="button"
              >
                Allow
              </Button>
            </div>
          </section>
        </div>
      ) : null}
      {descriptor ? (
        <div
          className="pointer-events-none absolute bottom-3 left-3 z-40 rounded-sm border border-border/70 bg-background/90 px-2 py-1 text-3xs font-medium text-muted-foreground shadow-sm"
          data-capabilities={capabilities.join(" ")}
          data-testid="project-canvas-capability-badge"
          title={
            capabilities.length > 0
              ? `Granted capabilities: ${capabilities.join(", ")}`
              : "No capabilities granted"
          }
        >
          Local Canvas
          {capabilities.length > 0
            ? ` · ${capabilities.length} ${
                capabilities.length === 1 ? "capability" : "capabilities"
              }`
            : ""}
        </div>
      ) : null}
    </section>
  );
}

function CanvasFailure({
  message,
  onReload,
}: {
  message: string;
  onReload: () => void;
}) {
  return (
    <div
      className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center"
      data-testid="project-canvas-error"
      role="alert"
    >
      <AlertTriangle className="h-6 w-6 text-destructive" />
      <div>
        <p className="text-sm font-medium">Canvas could not load</p>
        <p className="mt-1 max-w-lg text-xs text-muted-foreground">{message}</p>
      </div>
      <Button onClick={onReload} size="sm" type="button" variant="outline">
        <RefreshCw className="h-4 w-4" />
        Retry
      </Button>
    </div>
  );
}
