import { LoaderCircle } from "lucide-react";
import * as React from "react";

import type { ProjectCanvasBroker } from "./projectCanvasBroker";
import {
  commitProjectCanvasPackage,
  projectCanvasErrorMessage,
  releaseProjectCanvasPackage,
} from "./projectCanvasCommands";
import {
  isMessageWithinSizeLimit,
  parseProjectCanvasChildMessage,
  parseProjectCanvasReady,
  PROJECT_CANVAS_HANDSHAKE_TIMEOUT_MS,
  PROJECT_CANVAS_MAX_INIT_MESSAGE_BYTES,
  PROJECT_CANVAS_PROTOCOL_VERSION,
  ProjectCanvasMessageRateLimiter,
  selectGrantedProjectCanvasSnapshots,
  type ProjectCanvasCapability,
  type ProjectCanvasLayoutMessage,
  type ProjectCanvasLayouts,
  type ProjectCanvasPackageDescriptor,
  type ProjectCanvasPendingUpdates,
  type ProjectCanvasSnapshots,
} from "./projectCanvasProtocol";
import {
  createProjectCanvasRpcSession,
  type ProjectCanvasRpcSession,
} from "./projectCanvasRpc";

/**
 * The sandboxed canvas frame and its port session. Remounted per load id, so
 * every handshake, RPC session, and layout binding belongs to exactly one
 * activated package.
 */

export type ProjectCanvasMode = "preview" | "full";

const MAX_INVALID_PORT_MESSAGES = 3;

export function ProjectCanvasFrame({
  broker,
  capabilities,
  dataUpdate,
  descriptor,
  loadLayouts,
  mode,
  onCommandSettled,
  onFailure,
  onLayoutChanged,
  onRendered,
  projectId,
  projectName,
  projectNames,
  snapshots,
}: {
  broker: ProjectCanvasBroker | null;
  capabilities: readonly ProjectCanvasCapability[];
  dataUpdate: ProjectCanvasPendingUpdates["data"];
  descriptor: ProjectCanvasPackageDescriptor;
  loadLayouts: () => ProjectCanvasLayouts;
  mode: ProjectCanvasMode;
  onCommandSettled: (commandName: string, error: string | null) => void;
  onFailure: (loadId: string, message: string) => void;
  onLayoutChanged: (message: ProjectCanvasLayoutMessage) => void;
  onRendered: (loadId: string) => void;
  projectId: string;
  projectName: string;
  projectNames: readonly string[];
  snapshots: ProjectCanvasSnapshots;
}) {
  const frameRef = React.useRef<HTMLIFrameElement>(null);
  const portRef = React.useRef<MessagePort | null>(null);
  const rpcRef = React.useRef<ProjectCanvasRpcSession | null>(null);
  const modeRef = React.useRef(mode);
  const snapshotsRef = React.useRef(snapshots);
  const projectNameRef = React.useRef(projectName);
  const projectNamesRef = React.useRef(projectNames);
  const capabilitiesRef = React.useRef(capabilities);
  const brokerRef = React.useRef(broker);
  const loadLayoutsRef = React.useRef(loadLayouts);
  const onCommandSettledRef = React.useRef(onCommandSettled);
  const onLayoutChangedRef = React.useRef(onLayoutChanged);
  const connectedRef = React.useRef(false);
  const loadCountRef = React.useRef(0);
  const lastSnapshotsJsonRef = React.useRef<string | null>(null);
  const lastWidgetDataNotificationRef = React.useRef<string | null>(null);
  const [connected, setConnected] = React.useState(false);
  const [rendered, setRendered] = React.useState(false);
  const [failed, setFailed] = React.useState(false);
  const [frameSource, setFrameSource] = React.useState<string | undefined>();

  modeRef.current = mode;
  snapshotsRef.current = snapshots;
  projectNameRef.current = projectName;
  projectNamesRef.current = projectNames;
  capabilitiesRef.current = capabilities;
  brokerRef.current = broker;
  loadLayoutsRef.current = loadLayouts;
  onCommandSettledRef.current = onCommandSettled;
  onLayoutChangedRef.current = onLayoutChanged;

  const fail = React.useCallback(
    (message: string) => {
      connectedRef.current = false;
      rpcRef.current?.dispose();
      rpcRef.current = null;
      portRef.current?.close();
      portRef.current = null;
      setConnected(false);
      setRendered(false);
      setFailed(true);
      setFrameSource(undefined);
      void releaseProjectCanvasPackage(descriptor.loadId).catch(() => {});
      onFailure(descriptor.loadId, message);
    },
    [descriptor.loadId, onFailure],
  );

  React.useLayoutEffect(() => {
    const frameWindow = frameRef.current?.contentWindow;
    if (!frameWindow) {
      fail("Canvas frame could not be created.");
      return;
    }

    const grantedCapabilities = [...capabilitiesRef.current];
    const rateLimiter = new ProjectCanvasMessageRateLimiter();
    let invalidMessageCount = 0;
    let handshakeComplete = false;
    let renderAcknowledged = false;
    let stopped = false;
    let timeoutId = 0;
    const stop = (message: string) => {
      if (stopped) return;
      stopped = true;
      window.clearTimeout(timeoutId);
      fail(message);
    };
    timeoutId = window.setTimeout(() => {
      stop("Canvas did not complete its secure handshake and render.");
    }, PROJECT_CANVAS_HANDSHAKE_TIMEOUT_MS);

    const handleReady = (event: MessageEvent) => {
      if (stopped) return;
      if (event.source !== frameWindow) return;
      if (
        typeof event.data !== "object" ||
        event.data === null ||
        !("type" in event.data) ||
        event.data.type !== "canvas.ready"
      ) {
        return;
      }
      if (!parseProjectCanvasReady(event.data, descriptor.nonce)) {
        stop("Canvas sent an invalid handshake.");
        return;
      }
      if (handshakeComplete || connectedRef.current) {
        stop("Canvas attempted to reconnect unexpectedly.");
        return;
      }

      const channel = new MessageChannel();
      const grantedSnapshots = selectGrantedProjectCanvasSnapshots(
        snapshotsRef.current,
        grantedCapabilities,
      );
      const initMessage = {
        canvasId: projectId,
        capabilities: grantedCapabilities,
        data: descriptor.data,
        layouts: loadLayoutsRef.current(),
        loadId: descriptor.loadId,
        mode: modeRef.current,
        nonce: descriptor.nonce,
        project: {
          displayName: projectNameRef.current,
          id: projectId,
          name: projectNameRef.current,
          names: [...projectNamesRef.current].slice(0, 8),
        },
        protocolVersion: PROJECT_CANVAS_PROTOCOL_VERSION,
        snapshots: grantedSnapshots,
        type: "host.init",
      } as const;
      if (
        !isMessageWithinSizeLimit(
          initMessage,
          PROJECT_CANVAS_MAX_INIT_MESSAGE_BYTES,
        )
      ) {
        channel.port1.close();
        channel.port2.close();
        stop("Canvas initialization exceeds the host size limit.");
        return;
      }

      const rpcSession = createProjectCanvasRpcSession({
        broker: brokerRef.current,
        capabilities: grantedCapabilities,
        loadId: descriptor.loadId,
        nonce: descriptor.nonce,
        onCommandSettled: (commandName, commandError) => {
          if (!stopped) {
            onCommandSettledRef.current(commandName, commandError);
          }
        },
        post: (message) => {
          if (!stopped && portRef.current === channel.port1) {
            channel.port1.postMessage(message);
          }
        },
      });
      rpcRef.current = rpcSession;

      channel.port1.addEventListener("message", (portEvent) => {
        if (!rateLimiter.accept(performance.now())) {
          stop("Canvas exceeded the host message rate limit.");
          return;
        }
        const message = parseProjectCanvasChildMessage(portEvent.data, {
          loadId: descriptor.loadId,
          nonce: descriptor.nonce,
        });
        if (!message) {
          invalidMessageCount += 1;
          if (invalidMessageCount >= MAX_INVALID_PORT_MESSAGES) {
            stop("Canvas sent repeated invalid messages.");
          }
          return;
        }
        invalidMessageCount = 0;
        if (message.type === "canvas.rendered") {
          if (renderAcknowledged) {
            stop("Canvas reported completion more than once.");
            return;
          }
          renderAcknowledged = true;
          void commitProjectCanvasPackage(descriptor.loadId)
            .then(() => {
              if (stopped) return;
              window.clearTimeout(timeoutId);
              setRendered(true);
              onRendered(descriptor.loadId);
            })
            .catch((error: unknown) => {
              stop(projectCanvasErrorMessage(error));
            });
          return;
        }
        if (message.type === "canvas.layout") {
          // Host chrome state, not broker RPC: no capability, no consent, and
          // no rate tier beyond the shared port limiter.
          onLayoutChangedRef.current(message);
          return;
        }
        rpcSession.handle(message);
      });
      channel.port1.addEventListener("messageerror", () => {
        stop("Canvas sent an unreadable message.");
      });
      channel.port1.start();
      portRef.current = channel.port1;

      frameWindow.postMessage(
        {
          loadId: descriptor.loadId,
          nonce: descriptor.nonce,
          protocolVersion: PROJECT_CANVAS_PROTOCOL_VERSION,
          type: "host.connect",
        },
        "*",
        [channel.port2],
      );
      channel.port1.postMessage(initMessage);
      lastSnapshotsJsonRef.current = JSON.stringify(grantedSnapshots);
      handshakeComplete = true;
      connectedRef.current = true;
      setConnected(true);
    };

    window.addEventListener("message", handleReady);
    setFrameSource(descriptor.url);
    return () => {
      stopped = true;
      handshakeComplete = true;
      window.clearTimeout(timeoutId);
      window.removeEventListener("message", handleReady);
      connectedRef.current = false;
      rpcRef.current?.dispose();
      rpcRef.current = null;
      portRef.current?.close();
      portRef.current = null;
    };
  }, [descriptor, fail, onRendered, projectId]);

  React.useEffect(() => {
    if (!connectedRef.current || !portRef.current) return;
    portRef.current.postMessage({
      loadId: descriptor.loadId,
      mode,
      nonce: descriptor.nonce,
      protocolVersion: PROJECT_CANVAS_PROTOCOL_VERSION,
      type: "host.mode",
    });
  }, [descriptor.loadId, descriptor.nonce, mode]);

  React.useEffect(() => {
    const port = portRef.current;
    if (!connected || !port) return;
    const grantedSnapshots = selectGrantedProjectCanvasSnapshots(
      snapshots,
      capabilities,
    );
    const serialized = JSON.stringify(grantedSnapshots);
    if (serialized === lastSnapshotsJsonRef.current) return;
    const message = {
      loadId: descriptor.loadId,
      nonce: descriptor.nonce,
      protocolVersion: PROJECT_CANVAS_PROTOCOL_VERSION,
      snapshots: grantedSnapshots,
      type: "host.dataChanged",
    } as const;
    if (
      !isMessageWithinSizeLimit(message, PROJECT_CANVAS_MAX_INIT_MESSAGE_BYTES)
    ) {
      fail("Canvas data update exceeds the host size limit.");
      return;
    }
    port.postMessage(message);
    lastSnapshotsJsonRef.current = serialized;
  }, [capabilities, connected, descriptor, fail, snapshots]);

  React.useEffect(() => {
    const port = portRef.current;
    if (
      !connected ||
      !port ||
      !dataUpdate ||
      dataUpdate.notificationId === lastWidgetDataNotificationRef.current
    ) {
      return;
    }
    const message = {
      data: dataUpdate.data,
      loadId: descriptor.loadId,
      nonce: descriptor.nonce,
      notificationId: dataUpdate.notificationId,
      protocolVersion: PROJECT_CANVAS_PROTOCOL_VERSION,
      type: "host.widgetDataChanged",
      widgetId: dataUpdate.widgetId,
    } as const;
    if (
      !isMessageWithinSizeLimit(message, PROJECT_CANVAS_MAX_INIT_MESSAGE_BYTES)
    ) {
      fail("Canvas widget data update exceeds the host size limit.");
      return;
    }
    port.postMessage(message);
    lastWidgetDataNotificationRef.current = dataUpdate.notificationId;
  }, [connected, dataUpdate, descriptor.loadId, descriptor.nonce, fail]);

  return (
    <div className="relative h-full min-h-0 w-full bg-background">
      {!failed ? (
        <iframe
          allow="autoplay"
          className="h-full w-full border-0 bg-transparent"
          data-canvas-connected={connected ? "true" : "false"}
          data-canvas-rendered={rendered ? "true" : "false"}
          data-testid="project-canvas-frame"
          onError={() => fail("Canvas frame failed to load.")}
          onLoad={() => {
            if (!frameSource) return;
            loadCountRef.current += 1;
            if (loadCountRef.current > 1) {
              fail("Canvas navigated away from its host shell.");
            }
          }}
          ref={frameRef}
          referrerPolicy="no-referrer"
          sandbox="allow-scripts"
          src={frameSource}
          title={`${projectName} Canvas`}
        />
      ) : null}
      {!rendered && !failed ? (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-background/70 text-muted-foreground">
          <LoaderCircle
            aria-label="Connecting Canvas"
            className="h-5 w-5 animate-spin"
          />
        </div>
      ) : null}
    </div>
  );
}
