import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import * as React from "react";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_HUDDLE_CAMERA } from "@/shared/constants/kinds";
import { CameraController } from "./lib/cameraController";
import {
  acquireHuddleCamera,
  startCameraFramePump,
  type CameraDevice,
} from "./lib/huddleCamera";
import { useHuddleCameraDevices } from "./lib/useHuddleCameraDevices";
import { startScreenPublisher } from "./lib/screenPublisher";
import {
  ScreenShareReceiver,
  type ScreenShare,
  type ScreenVideoStream,
} from "./lib/screenShare";

type CameraState = {
  phase: string;
  ephemeral_channel_id: string | null;
  camera_share?: ScreenShare | null;
  camera_device_id?: string;
  camera_error?: string | null;
};
type CameraRequest = {
  channel: string;
  action: "start" | "stop";
  deviceId: string;
  label: string;
};
const EMPTY_STREAMS = new Map<string, ScreenVideoStream>();
type CameraContextValue = {
  channelId: string | null;
  localShare: ScreenShare | null;
  selectedDeviceId: string;
  cameras: CameraDevice[];
  checking: boolean;
  busy: boolean;
  error: string | null;
  streams: ReadonlyMap<string, ScreenVideoStream>;
  refresh: () => void;
  request: (
    action: "start" | "stop",
    deviceId?: string,
    label?: string,
  ) => Promise<void>;
};
const CameraContext = React.createContext<CameraContextValue>({
  channelId: null,
  localShare: null,
  selectedDeviceId: "",
  cameras: [],
  checking: true,
  busy: false,
  error: null,
  streams: EMPTY_STREAMS,
  refresh: () => {},
  request: async () => {},
});

/** Webcam controls and independently scoped live participant video. */
export function useHuddleCamera() {
  return React.useContext(CameraContext);
}

/** The main window owns the permission, track, frame pump, and publisher. */
export function HuddleCameraProvider({
  channelId: ownerChannel,
  ownsCapture,
  children,
}: {
  channelId: string | null;
  ownsCapture: boolean;
  children: React.ReactNode;
}) {
  const [backend, setBackend] = React.useState<CameraState | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [streams, setStreams] = React.useState<ScreenVideoStream[]>([]);
  const channel = ownsCapture
    ? ownerChannel
    : (backend?.ephemeral_channel_id ?? null);
  const active =
    !!channel &&
    backend?.ephemeral_channel_id === channel &&
    (backend.phase === "active" || backend.phase === "connected");
  const {
    cameras,
    checking,
    error: deviceError,
    refresh,
  } = useHuddleCameraDevices(active ? channel : null, ownsCapture);
  const scope = React.useRef({ channel, active });
  scope.current = { channel, active };
  const controller = React.useRef<CameraController | null>(null);
  const current = React.useCallback(
    (id: string) => scope.current.active && scope.current.channel === id,
    [],
  );

  React.useEffect(() => {
    let disposed = false;
    let revision = 0;
    let cleanup: (() => void) | undefined;
    void listen<CameraState>("huddle-state-changed", ({ payload }) => {
      if (!disposed) {
        revision++;
        setBackend(payload);
      }
    })
      .then(async (unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        cleanup = unlisten;
        const fetched = revision;
        const state = await invoke<CameraState>("get_huddle_state");
        if (!disposed && revision === fetched) {
          setBackend(state);
          // A reloaded owner cannot inherit browser capture from a destroyed page.
          if (
            ownsCapture &&
            state.camera_share &&
            !controller.current?.sessionId
          ) {
            await invoke("stop_huddle_camera", {
              sessionId: state.camera_share.session_id,
            });
          }
        }
      })
      .catch((failure) => {
        if (!disposed) setError(String(failure));
      });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [ownsCapture]);

  React.useEffect(() => {
    if (!ownsCapture) return;
    const owner = new CameraController({
      current,
      reserve: (channelId, deviceId, label) =>
        invoke<ScreenShare>("start_huddle_camera", {
          channelId,
          deviceId,
          label,
        }),
      retire: (sessionId, failure) =>
        invoke("stop_huddle_camera", { sessionId, error: failure ?? null }),
      notifyStop: async (share, channelId) => {
        if (!current(channelId)) return;
        const event = await signRelayEvent({
          kind: KIND_HUDDLE_CAMERA,
          tags: [["h", channelId]],
          content: JSON.stringify({ type: "stopped", share }),
        });
        await relayClient.publishEvent(
          event,
          "Camera stop timed out",
          "Could not notify camera viewers",
          () => current(channelId),
        );
      },
      acquire: async (deviceId, alive) => {
        const stream = await acquireHuddleCamera(
          navigator.mediaDevices,
          deviceId,
          alive,
        );
        void refresh();
        return stream;
      },
      pump: (stream, share, alive, failed) =>
        startCameraFramePump({
          stream,
          isCurrent: alive,
          fail: failed,
          submit: (bytes, width, height) =>
            invoke("push_huddle_camera_frame", bytes, {
              headers: {
                "x-buzz-camera-session": share.session_id,
                "x-buzz-camera-width": String(width),
                "x-buzz-camera-height": String(height),
              },
            }),
        }),
      publish: (share, alive, failed) =>
        startScreenPublisher({
          capture: async () => {
            const json = await invoke<string | null>(
              "read_huddle_camera_video_chunk",
              { sessionId: share.session_id },
            );
            return json ? (JSON.parse(json) as RelayEvent) : null;
          },
          publish: (event, publisherAlive) =>
            relayClient.publishEvent(
              event,
              "Camera publication timed out",
              "Could not publish camera video",
              () => publisherAlive() && alive(),
            ),
          fail: async (failure) => failed(failure),
        }),
      changed: setBusy,
      failed: (failure) => setError(String(failure)),
    });
    controller.current = owner;
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void listen<CameraRequest>("huddle-camera-request", ({ payload }) => {
      if (disposed || !current(payload.channel)) return;
      setError(null);
      const operation =
        payload.action === "stop"
          ? owner.stop()
          : owner.start(payload.channel, payload.deviceId, payload.label);
      void operation.catch((failure) => {
        if (!disposed) {
          setError(String(failure));
          void emit("huddle-camera-error", {
            channel: payload.channel,
            error: String(failure),
          });
        }
      });
    })
      .then((unlisten) => {
        if (disposed) unlisten();
        else cleanup = unlisten;
      })
      .catch((failure) => {
        if (!disposed) setError(String(failure));
      });
    return () => {
      disposed = true;
      cleanup?.();
      if (controller.current === owner) controller.current = null;
      void owner
        .stop()
        .catch((failure) => console.error("Could not retire camera", failure));
    };
  }, [ownsCapture, current, refresh]);

  React.useEffect(() => {
    if (!ownsCapture) return;
    const owner = controller.current;
    if (
      !active ||
      (owner?.sessionId && owner.channelId !== channel) ||
      (owner?.sessionId &&
        backend?.camera_share?.session_id !== owner.sessionId)
    ) {
      void owner?.stop().catch((failure) => setError(String(failure)));
    }
  }, [active, backend?.camera_share?.session_id, channel, ownsCapture]);

  React.useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void listen<{ channel: string; error: string }>(
      "huddle-camera-error",
      ({ payload }) => {
        if (!disposed && current(payload.channel)) setError(payload.error);
      },
    )
      .then((unlisten) => {
        if (disposed) unlisten();
        else cleanup = unlisten;
      })
      .catch((failure) => {
        if (!disposed) setError(String(failure));
      });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [current]);

  React.useEffect(() => {
    setStreams([]);
    if (!active || !channel) return;
    let disposed = false;
    const receiver = new ScreenShareReceiver(channel, KIND_HUDDLE_CAMERA);
    const abort = new AbortController();
    const sync = () => {
      if (!disposed) setStreams(receiver.snapshot());
    };
    const clear = () => {
      receiver.clear();
      sync();
    };
    void relayClient
      .subscribeLive(
        { kinds: [KIND_HUDDLE_CAMERA], "#h": [channel], limit: 0 },
        (event) => {
          if (!disposed && receiver.receive(event)) sync();
        },
        undefined,
        undefined,
        abort.signal,
      )
      .catch((failure) => {
        if (!disposed)
          setError(`Could not receive camera video: ${String(failure)}`);
      });
    const connection = relayClient.subscribeToConnectionState((state) => {
      if (state !== "connected") {
        clear();
        if (ownsCapture)
          void controller.current
            ?.stop("Camera stopped because the relay disconnected")
            .catch((failure) => setError(String(failure)));
      }
    });
    const access = relayClient.subscribeToChannelAccessRevocations(() => {
      clear();
      if (ownsCapture)
        void controller.current
          ?.stop("Camera stopped because channel access changed")
          .catch((failure) => setError(String(failure)));
    }, channel);
    const timer = window.setInterval(() => {
      if (receiver.expire()) sync();
    }, 500);
    return () => {
      disposed = true;
      abort.abort();
      connection();
      access();
      window.clearInterval(timer);
      receiver.clear();
    };
  }, [active, channel, ownsCapture]);

  const request = React.useCallback(
    async (action: "start" | "stop", deviceId = "", label = "Camera") => {
      if (!scope.current.active || !scope.current.channel) return;
      setError(null);
      if (ownsCapture && controller.current) {
        try {
          if (action === "stop") await controller.current.stop();
          else
            await controller.current.start(
              scope.current.channel,
              deviceId,
              label,
            );
        } catch (failure) {
          setError(String(failure));
        }
      } else {
        await emit("huddle-camera-request", {
          action,
          channel: scope.current.channel,
          deviceId,
          label,
        });
      }
    },
    [ownsCapture],
  );
  const streamMap = React.useMemo(
    () => new Map(streams.map((stream) => [stream.pubkey, stream])),
    [streams],
  );
  const value = React.useMemo(
    () => ({
      channelId: active ? channel : null,
      localShare: active ? (backend?.camera_share ?? null) : null,
      selectedDeviceId: backend?.camera_device_id ?? "",
      cameras,
      checking,
      busy,
      error: backend?.camera_error ?? error ?? deviceError,
      streams: active ? streamMap : EMPTY_STREAMS,
      refresh,
      request,
    }),
    [
      active,
      channel,
      backend,
      cameras,
      checking,
      busy,
      error,
      deviceError,
      streamMap,
      refresh,
      request,
    ],
  );
  return (
    <CameraContext.Provider value={value}>{children}</CameraContext.Provider>
  );
}
