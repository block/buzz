import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  onAction,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { isLinuxPlatform, isMacPlatform } from "@/shared/lib/platform";

// Backend event emitted when a native Linux notification is clicked or a
// queued macOS activation becomes available. See src-tauri notification code.
const NATIVE_NOTIFICATION_ACTIVATED_EVENT = "native-notification-activated";
const TAKE_PENDING_MACOS_NOTIFICATION_ACTIVATIONS = "take_pending_activations";
const MACOS_NOTIFICATION_PERMISSION_STATE = "notification_permission_state";
const REQUEST_MACOS_NOTIFICATION_ACCESS = "request_notification_access";

export type DesktopNotificationPermissionState =
  | NotificationPermission
  | "unsupported";

export type AppBadgeState =
  | { kind: "none" }
  | { kind: "dot" }
  | { kind: "count"; count: number };

export type DesktopNotificationTarget = {
  channelId: string | null;
  channelName?: string | null;
  content?: string;
  createdAt?: number | null;
  eventId: string | null;
  kind: number | null;
  pubkey?: string;
  threadRootId?: string | null;
};

type DesktopNotificationPayload = {
  body?: string;
  target?: DesktopNotificationTarget;
  title: string;
};

const DESKTOP_NOTIFICATION_ACTION_EVENT = "buzz:desktop-notification-action";

type DesktopNotificationOptions = NotificationOptions & {
  extra?: Record<string, unknown>;
};

type TestWindow = Window & {
  __BUZZ_E2E_APP_BADGE_COUNT__?: number;
  __BUZZ_E2E_APP_BADGE_STATE__?: AppBadgeState["kind"];
};

function hasNotificationApi() {
  return typeof window !== "undefined" && "Notification" in window;
}

function notificationExtra(
  target: DesktopNotificationTarget | undefined,
): Record<string, unknown> | undefined {
  if (!target) {
    return undefined;
  }

  return {
    buzzNotificationTarget: target,
  };
}

function parseNotificationTarget(
  value: unknown,
): DesktopNotificationTarget | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const candidate = value as Partial<DesktopNotificationTarget>;
  const channelId =
    typeof candidate.channelId === "string" ? candidate.channelId : null;
  const channelName =
    typeof candidate.channelName === "string" ? candidate.channelName : null;
  const content =
    typeof candidate.content === "string" ? candidate.content : undefined;
  const createdAt =
    typeof candidate.createdAt === "number" ? candidate.createdAt : null;
  const eventId =
    typeof candidate.eventId === "string" ? candidate.eventId : null;
  const kind = typeof candidate.kind === "number" ? candidate.kind : null;
  const pubkey =
    typeof candidate.pubkey === "string" ? candidate.pubkey : undefined;
  const threadRootId =
    typeof candidate.threadRootId === "string" ? candidate.threadRootId : null;

  if (!channelId && !eventId) {
    return null;
  }

  return {
    channelId,
    channelName,
    content,
    createdAt,
    eventId,
    kind,
    pubkey,
    threadRootId,
  };
}

function dispatchDesktopNotificationTarget(target: DesktopNotificationTarget) {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(
    new CustomEvent<DesktopNotificationTarget>(
      DESKTOP_NOTIFICATION_ACTION_EVENT,
      {
        detail: target,
      },
    ),
  );
}

function shouldUseMacDevelopmentFallback(error: unknown): boolean {
  return String(error).includes("not running from an app bundle");
}

export async function getDesktopNotificationPermissionState(): Promise<DesktopNotificationPermissionState> {
  if (!hasNotificationApi()) {
    return "unsupported";
  }

  if (isTauri() && isMacPlatform()) {
    try {
      return await invoke<NotificationPermission>(
        MACOS_NOTIFICATION_PERMISSION_STATE,
      );
    } catch (error) {
      // The native API rejects the unbundled executable used by `tauri dev`.
      // Preserve that development path through the plugin-backed shim.
      if (!shouldUseMacDevelopmentFallback(error)) {
        return "default";
      }
    }
  }

  if (window.Notification.permission !== "default") {
    return window.Notification.permission;
  }

  if (!isTauri()) {
    return "default";
  }

  try {
    return (await isPermissionGranted()) ? "granted" : "default";
  } catch {
    return "default";
  }
}

let pendingPermissionRequest: Promise<DesktopNotificationPermissionState> | null =
  null;

export async function requestDesktopNotificationAccess(): Promise<DesktopNotificationPermissionState> {
  if (!hasNotificationApi()) {
    return "unsupported";
  }

  if (pendingPermissionRequest) {
    return pendingPermissionRequest;
  }

  const request =
    isTauri() && isMacPlatform()
      ? invoke<NotificationPermission>(REQUEST_MACOS_NOTIFICATION_ACCESS).catch(
          (error) => {
            if (shouldUseMacDevelopmentFallback(error)) {
              return requestPermission();
            }
            throw error;
          },
        )
      : requestPermission();
  pendingPermissionRequest = request.finally(() => {
    pendingPermissionRequest = null;
  });

  return pendingPermissionRequest;
}

export async function listenForDesktopNotificationActions(
  onTarget: (target: DesktopNotificationTarget) => void,
): Promise<() => void> {
  if (typeof window === "undefined") {
    return () => {};
  }

  function handleNotificationAction(event: Event) {
    const customEvent = event as CustomEvent<DesktopNotificationTarget>;
    onTarget(customEvent.detail);
  }

  window.addEventListener(
    DESKTOP_NOTIFICATION_ACTION_EVENT,
    handleNotificationAction,
  );

  let pluginListener: { unregister: () => Promise<void> } | null = null;
  let nativeUnlisten: (() => void) | null = null;

  if (isTauri()) {
    const usesMacActivationQueue = isMacPlatform();

    if (!isLinuxPlatform() && !usesMacActivationQueue) {
      try {
        pluginListener = await onAction((notification) => {
          const target = parseNotificationTarget(
            notification.extra?.buzzNotificationTarget,
          );
          if (!target) {
            return;
          }

          dispatchDesktopNotificationTarget(target);
        });
      } catch {
        pluginListener = null;
      }
    }

    // Linux forwards the target as the event payload. macOS queues targets in
    // Rust first so cold-start clicks survive until this listener is mounted.
    const dispatchNativeActivations = async (payload?: unknown) => {
      if (usesMacActivationQueue) {
        const targets = await invoke<unknown[]>(
          TAKE_PENDING_MACOS_NOTIFICATION_ACTIVATIONS,
        );
        for (const pendingTarget of targets) {
          const target = parseNotificationTarget(pendingTarget);
          if (target) {
            dispatchDesktopNotificationTarget(target);
          }
        }
        return;
      }

      const target = parseNotificationTarget(payload);
      if (target) {
        dispatchDesktopNotificationTarget(target);
      }
    };

    try {
      nativeUnlisten = await listen<unknown>(
        NATIVE_NOTIFICATION_ACTIVATED_EVENT,
        (event) => {
          void dispatchNativeActivations(event.payload).catch((error) => {
            console.error(
              "Failed to dispatch native notification activation",
              error,
            );
          });
        },
      );
    } catch {
      nativeUnlisten = null;
    }

    if (nativeUnlisten && usesMacActivationQueue) {
      try {
        await dispatchNativeActivations();
      } catch (error) {
        console.error(
          "Failed to drain pending macOS notification activations",
          error,
        );
      }
    }
  }

  return () => {
    window.removeEventListener(
      DESKTOP_NOTIFICATION_ACTION_EVENT,
      handleNotificationAction,
    );
    void pluginListener?.unregister();
    nativeUnlisten?.();
  };
}

export async function setDesktopAppBadge(state: AppBadgeState): Promise<void> {
  if (typeof window !== "undefined") {
    const testWindow = window as TestWindow;
    testWindow.__BUZZ_E2E_APP_BADGE_COUNT__ =
      state.kind === "count" ? state.count : 0;
    testWindow.__BUZZ_E2E_APP_BADGE_STATE__ = state.kind;
  }

  if (!isTauri()) {
    return;
  }

  try {
    if (state.kind === "count") {
      await getCurrentWindow().setBadgeCount(state.count);
    } else if (state.kind === "dot" && isMacPlatform()) {
      await getCurrentWindow().setBadgeLabel(" ");
    } else {
      if (isMacPlatform()) {
        await getCurrentWindow().setBadgeLabel("");
      }
      await getCurrentWindow().setBadgeCount(undefined);
    }
  } catch {
    // Ignore unsupported platforms and best-effort badge sync failures.
  }
}

export async function requestDockBounce(): Promise<void> {
  if (!isTauri()) {
    return;
  }
  if (document.hasFocus()) {
    return;
  }
  try {
    await getCurrentWindow().requestUserAttention(
      UserAttentionType.Informational,
    );
  } catch {
    // Best effort; ignore unsupported platforms.
  }
}

export async function revealDesktopAppWindow(): Promise<void> {
  if (!isTauri()) {
    if (typeof window !== "undefined") {
      window.focus();
    }
    return;
  }

  try {
    const currentWindow = getCurrentWindow();
    await currentWindow.unminimize();
    await currentWindow.show();
    await currentWindow.setFocus();
  } catch {
    // Best effort only.
  }
}

export async function sendDesktopNotification(
  payload: DesktopNotificationPayload,
): Promise<boolean> {
  if ((await getDesktopNotificationPermissionState()) !== "granted") {
    return false;
  }

  // Linux needs a retained D-Bus connection. macOS needs a native notification
  // center delegate because the Tauri plugin does not deliver desktop clicks.
  // See src-tauri/src/commands/notifications.rs.
  if (isTauri() && (isLinuxPlatform() || isMacPlatform())) {
    try {
      await invoke("show_native_notification", {
        title: payload.title,
        body: payload.body,
        target: payload.target ?? null,
      });
      return true;
    } catch {
      if (!isMacPlatform()) {
        return false;
      }
      // UNUserNotificationCenter is unavailable to the unbundled executable
      // used by Tauri dev. Preserve the previous macOS development behavior by
      // falling through to the notification plugin; packaged apps use native UN.
    }
  }

  const notification = new window.Notification(payload.title, {
    body: payload.body,
    silent: true,
    extra: notificationExtra(payload.target),
  } as DesktopNotificationOptions);

  const target = payload.target;
  if (!isTauri() && target) {
    notification.onclick = () => {
      dispatchDesktopNotificationTarget(target);
      notification.close();
    };
  }

  return true;
}
