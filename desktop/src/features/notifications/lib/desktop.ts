import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  onAction,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { isLinuxPlatform, isMacPlatform } from "@/shared/lib/platform";

// Backend event emitted when the user clicks a native (Linux) notification.
// See src-tauri/src/commands/notifications.rs.
const NATIVE_NOTIFICATION_ACTIVATED_EVENT = "native-notification-activated";

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

export async function getDesktopNotificationPermissionState(): Promise<DesktopNotificationPermissionState> {
  if (!hasNotificationApi()) {
    return "unsupported";
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

  pendingPermissionRequest = requestPermission().finally(() => {
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

    // Clicks on Linux notifications come back via a backend event rather than
    // the plugin's onAction (whose connection is torn down before it can fire).
    try {
      nativeUnlisten = await listen<unknown>(
        NATIVE_NOTIFICATION_ACTIVATED_EVENT,
        (event) => {
          const target = parseNotificationTarget(event.payload);
          if (!target) {
            return;
          }

          dispatchDesktopNotificationTarget(target);
        },
      );
    } catch {
      nativeUnlisten = null;
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

  // On Linux the bundled notification plugin posts via a D-Bus connection that
  // it drops immediately; GNOME 46+ then dismisses the notification before it
  // is seen. Route through a backend command that keeps the connection alive.
  // See src-tauri/src/commands/notifications.rs.
  if (isTauri() && isLinuxPlatform()) {
    try {
      await invoke("show_native_notification", {
        title: payload.title,
        body: payload.body,
        target: payload.target ?? null,
      });
      return true;
    } catch {
      return false;
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
