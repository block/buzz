import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  onAction,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import {
  isLinuxPlatform,
  isMacPlatform,
  isWindowsPlatform,
} from "@/shared/lib/platform";

// Backend event emitted when a native Linux notification is clicked or a
// queued macOS activation becomes available. See src-tauri notification code.
const NATIVE_NOTIFICATION_ACTIVATED_EVENT = "native-notification-activated";
const TAKE_PENDING_MACOS_NOTIFICATION_ACTIVATIONS = "take_pending_activations";
const TAKE_PENDING_WINDOWS_NOTIFICATION_ACTIVATIONS =
  "take_pending_windows_activations";
const MACOS_NOTIFICATION_PERMISSION_STATE = "notification_permission_state";
const REQUEST_MACOS_NOTIFICATION_ACCESS = "request_notification_access";
const WINDOWS_NOTIFICATION_PERMISSION_STATE =
  "windows_notification_permission_state";

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
  openInThread?: boolean;
  threadRootId?: string | null;
};

export type DesktopNotificationPayload = {
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
  // Notifications can outlive an app upgrade. Legacy payloads did not carry
  // openInThread, so only those infer branch navigation from their root id.
  const openInThread =
    typeof candidate.openInThread === "boolean"
      ? candidate.openInThread
      : threadRootId !== null;

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
    openInThread,
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

  // Windows has enabled/disabled toast settings, not a browser-style prompt
  // lifecycle. Query the AppUserModelID's native ToastNotifier setting so a
  // system block remains terminal until the user changes Windows Settings.
  if (isTauri() && isWindowsPlatform()) {
    return invoke<NotificationPermission>(
      WINDOWS_NOTIFICATION_PERMISSION_STATE,
    );
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
    isTauri() && isWindowsPlatform()
      ? getDesktopNotificationPermissionState()
      : isTauri() && isMacPlatform()
        ? invoke<NotificationPermission>(
            REQUEST_MACOS_NOTIFICATION_ACCESS,
          ).catch((error) => {
            if (shouldUseMacDevelopmentFallback(error)) {
              return requestPermission();
            }
            throw error;
          })
        : requestPermission();
  pendingPermissionRequest = request.finally(() => {
    pendingPermissionRequest = null;
  });

  return pendingPermissionRequest;
}

export async function ensureDesktopNotificationPermissionGranted(
  getPermissionState = getDesktopNotificationPermissionState,
  requestAccess = requestDesktopNotificationAccess,
): Promise<boolean> {
  try {
    const currentPermission = await getPermissionState();
    if (currentPermission === "granted") {
      return true;
    }
    if (currentPermission !== "default") {
      return false;
    }
    return (await requestAccess()) === "granted";
  } catch (error) {
    console.warn("Failed to determine desktop notification permission", error);
    return false;
  }
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
  let redrainUnlisten: (() => void) | null = null;

  if (isTauri()) {
    const usesMacActivationQueue = isMacPlatform();
    const usesWindowsActivationQueue = isWindowsPlatform();
    const usesActivationQueue =
      usesMacActivationQueue || usesWindowsActivationQueue;

    // Keep the plugin action path for other Tauri targets, such as Android;
    // Linux, macOS, and Windows use the native event/activation paths above.
    if (!isLinuxPlatform() && !usesActivationQueue) {
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

    // Linux forwards the target as the event payload. macOS and Windows queue
    // targets in Rust before emitting so clicks survive a missing listener.
    const dispatchNativeActivations = async (payload?: unknown) => {
      if (usesActivationQueue) {
        const targets = await invoke<unknown[]>(
          usesMacActivationQueue
            ? TAKE_PENDING_MACOS_NOTIFICATION_ACTIVATIONS
            : TAKE_PENDING_WINDOWS_NOTIFICATION_ACTIVATIONS,
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

    if (usesActivationQueue) {
      try {
        await dispatchNativeActivations();
      } catch (error) {
        console.error(
          "Failed to drain pending notification activations",
          error,
        );
      }
    }

    if (usesActivationQueue) {
      // Belt and suspenders for block/buzz#3509: the Rust delegate queues the
      // target before emitting, so a lost emit strands the activation with
      // nothing re-draining it. macOS always foregrounds the app on a
      // notification click, and WebKit delivers the resulting focus and
      // visibility transitions independently of the Tauri event channel — use
      // them to re-drain so a queued target is never stranded.
      const redrain = () => {
        void dispatchNativeActivations().catch((error) => {
          console.error(
            "Failed to drain pending notification activations on focus",
            error,
          );
        });
      };
      window.addEventListener("focus", redrain);
      document.addEventListener("visibilitychange", redrain);
      redrainUnlisten = () => {
        window.removeEventListener("focus", redrain);
        document.removeEventListener("visibilitychange", redrain);
      };
    }
  }

  return () => {
    window.removeEventListener(
      DESKTOP_NOTIFICATION_ACTION_EVENT,
      handleNotificationAction,
    );
    void pluginListener?.unregister();
    nativeUnlisten?.();
    redrainUnlisten?.();
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

/**
 * How long the window-reveal invoke chain may run before callers proceed
 * without it. macOS already foregrounds the app when a notification is
 * clicked, so a reveal that never settles must not gate click-through
 * routing (block/buzz#3509).
 */
const REVEAL_WINDOW_TIMEOUT_MS = 1_500;

function resolveWithinTimeout(
  operation: Promise<void>,
  timeoutMs: number,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(resolve, timeoutMs);
    operation.then(
      () => {
        clearTimeout(timer);
        resolve();
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
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
    // The reveal crosses the IPC boundary three times, and the try/catch
    // only covers rejections — an invoke that never settles (seen while
    // macOS is simultaneously foregrounding the app from a notification
    // click) would strand callers that await this helper before navigating.
    // Resolve after a timeout so navigation always proceeds.
    await resolveWithinTimeout(
      (async () => {
        await currentWindow.unminimize();
        await currentWindow.show();
        await currentWindow.setFocus();
      })(),
      REVEAL_WINDOW_TIMEOUT_MS,
    );
  } catch {
    // Best effort only.
  }
}

export async function sendDesktopNotification(
  payload: DesktopNotificationPayload,
  canDeliver: () => boolean = () => true,
): Promise<boolean> {
  let permission: DesktopNotificationPermissionState;
  try {
    permission = await getDesktopNotificationPermissionState();
  } catch (error) {
    console.warn("Failed to determine desktop notification permission", error);
    return false;
  }

  if (permission !== "granted" || !canDeliver()) {
    return false;
  }

  // Linux needs a retained D-Bus connection. macOS needs a native notification
  // center delegate because the Tauri plugin does not deliver desktop clicks.
  // Windows needs WinRT toast notifications so the app registers with
  // Settings > System > Notifications and click actions work.
  // Do NOT use the Tauri notification plugin's sendNotification() on Windows —
  // the native WinRT path handles delivery and click actions exclusively.
  // See src-tauri/src/commands/notifications.rs.
  if (
    isTauri() &&
    (isLinuxPlatform() || isMacPlatform() || isWindowsPlatform())
  ) {
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

  // block/buzz#5081 — WebKit throws `NotificationError` from the constructor
  // when the notification backend becomes temporarily unavailable. Callers
  // discard the returned promise without a rejection handler, so an
  // un-guarded throw becomes an unhandled rejection. Treat constructor failure
  // as a delivery miss (return false) and log the failed delivery.
  try {
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
  } catch (error) {
    console.warn(
      "[desktop] window.Notification constructor threw — notification dropped:",
      error,
    );
    return false;
  }
}
