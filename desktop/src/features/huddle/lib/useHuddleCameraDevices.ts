import { emit, listen } from "@tauri-apps/api/event";
import * as React from "react";
import { type CameraDevice, listHuddleCameras } from "./huddleCamera";

type Result = { cameras: CameraDevice[] } | { error: string };
type DeviceMessage =
  | { channel: string; type: "ready" }
  | { channel: string; type: "request"; requestId: string }
  | { channel: string; type: "result"; requestId: string; result: Result };
const DEVICE_EVENT = "huddle-camera-devices";

/** Demand-driven discovery avoids WebKit's enumeration/devicechange feedback loop. */
export function useHuddleCameraDevices(
  channelId: string | null,
  ownsCapture: boolean,
) {
  const [cameras, setCameras] = React.useState<CameraDevice[]>([]);
  const [checking, setChecking] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const scope = React.useRef(channelId);
  scope.current = channelId;
  const mounted = React.useRef(false);
  const generation = React.useRef(0);
  const pending = React.useRef<Promise<CameraDevice[]> | null>(null);
  const remote = React.useRef<{
    id: string;
    complete: (result: Result | null) => void;
  } | null>(null);
  const apply = React.useCallback((result: Result) => {
    if ("error" in result) {
      setError(result.error);
      return;
    }
    const found = result.cameras;
    setCameras((previous) =>
      previous.length === found.length &&
      previous.every(
        (camera, index) =>
          camera.deviceId === found[index].deviceId &&
          camera.label === found[index].label,
      )
        ? previous
        : found,
    );
    setError(null);
  }, []);

  const refresh = React.useCallback(async (): Promise<Result | null> => {
    const channel = scope.current;
    if (!mounted.current || !channel) return null;
    const request = ++generation.current;
    const current = () =>
      mounted.current &&
      scope.current === channel &&
      generation.current === request;
    setChecking(true);
    if (!ownsCapture) {
      remote.current?.complete(null);
      const requestId = crypto.randomUUID();
      return new Promise((resolve) => {
        let completed = false;
        const complete = (result: Result | null) => {
          if (completed) return;
          completed = true;
          clearTimeout(timer);
          if (remote.current?.id === requestId) remote.current = null;
          if (current() && result) {
            apply(result);
            setChecking(false);
          }
          resolve(result);
        };
        const timer = setTimeout(
          () =>
            complete({
              error:
                "Could not list cameras: the capture window did not respond. Try Refresh cameras.",
            }),
          5000,
        );
        remote.current = { id: requestId, complete };
        void emit<DeviceMessage>(DEVICE_EVENT, {
          channel,
          type: "request",
          requestId,
        }).catch((failure) =>
          complete({ error: `Could not list cameras: ${String(failure)}` }),
        );
      });
    }
    const devices = navigator.mediaDevices;
    if (!devices?.enumerateDevices) {
      const result = { cameras: [] };
      apply(result);
      setChecking(false);
      return result;
    }
    const probe = pending.current ?? listHuddleCameras(devices);
    pending.current = probe;
    try {
      const found = await probe;
      const result = { cameras: found };
      if (current()) apply(result);
      return result;
    } catch (failure) {
      const result = { error: `Could not list cameras: ${String(failure)}` };
      if (current()) apply(result);
      return result;
    } finally {
      if (pending.current === probe) pending.current = null;
      if (current()) setChecking(false);
    }
  }, [apply, ownsCapture]);

  React.useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let cleanup: (() => void) | undefined;
    if (channelId) {
      void listen<DeviceMessage>(DEVICE_EVENT, ({ payload }) => {
        if (disposed || payload.channel !== scope.current) return;
        if (ownsCapture && payload.type === "request") {
          void refresh()
            .then(async (result) => {
              if (!disposed && scope.current === payload.channel && result) {
                await emit<DeviceMessage>(DEVICE_EVENT, {
                  channel: payload.channel,
                  type: "result",
                  requestId: payload.requestId,
                  result,
                });
              }
            })
            .catch((failure) => {
              if (!disposed)
                setError(`Could not share camera devices: ${String(failure)}`);
            });
        } else if (!ownsCapture && payload.type === "ready") {
          void refresh();
        } else if (
          !ownsCapture &&
          payload.type === "result" &&
          remote.current?.id === payload.requestId
        ) {
          remote.current.complete(payload.result);
        }
      })
        .then((unlisten) => {
          if (disposed) {
            unlisten();
            return;
          }
          cleanup = unlisten;
          void refresh();
          if (ownsCapture) {
            void emit<DeviceMessage>(DEVICE_EVENT, {
              channel: channelId,
              type: "ready",
            }).catch((failure) => {
              if (!disposed)
                setError(
                  `Could not announce camera devices: ${String(failure)}`,
                );
            });
          }
        })
        .catch((failure) => {
          if (!disposed) {
            setChecking(false);
            setError(`Could not discover cameras: ${String(failure)}`);
          }
        });
    } else {
      generation.current++;
      setCameras([]);
      setChecking(false);
      setError(null);
    }
    return () => {
      disposed = true;
      mounted.current = false;
      generation.current++;
      remote.current?.complete(null);
      cleanup?.();
    };
  }, [channelId, ownsCapture, refresh]);

  return { cameras, checking, error, refresh };
}
