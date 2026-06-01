import { isTauri } from "@tauri-apps/api/core";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  onAction,
  requestPermission,
} from "@tauri-apps/plugin-notification";

export type DesktopNotificationPermissionState =
  | NotificationPermission
  | "unsupported";

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

const DESKTOP_NOTIFICATION_ACTION_EVENT = "sprout:desktop-notification-action";

type DesktopNotificationOptions = NotificationOptions & {
  extra?: Record<string, unknown>;
};

type TestWindow = Window & {
  __SPROUT_E2E_APP_BADGE_COUNT__?: number;
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
    sproutNotificationTarget: target,
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

  if (isTauri()) {
    try {
      pluginListener = await onAction((notification) => {
        const target = parseNotificationTarget(
          notification.extra?.sproutNotificationTarget,
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

  return () => {
    window.removeEventListener(
      DESKTOP_NOTIFICATION_ACTION_EVENT,
      handleNotificationAction,
    );
    void pluginListener?.unregister();
  };
}

export async function setDesktopAppBadgeCount(count: number): Promise<void> {
  if (typeof window !== "undefined") {
    (window as TestWindow).__SPROUT_E2E_APP_BADGE_COUNT__ = count;
  }

  if (!isTauri()) {
    return;
  }

  try {
    await getCurrentWindow().setBadgeCount(count > 0 ? count : undefined);
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
