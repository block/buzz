import * as React from "react";
import {
  ensureDesktopNotificationPermissionGranted,
  sendDesktopNotification,
  type DesktopNotificationPayload,
} from "./lib/desktop";

export type LiveNotification = {
  id: string;
  slot: "dm" | "thread_reply";
  payload: DesktopNotificationPayload;
};

type PendingNotification = {
  notification: LiveNotification;
  attempts: number;
  due: number;
};
type DeliverySession = {
  key: string;
  pending: Map<string, PendingNotification>;
  timer?: ReturnType<typeof setTimeout>;
  running: boolean;
  disposed: boolean;
  generation: number;
};

const MAX_PENDING = 500;
const MAX_ATTEMPTS = 5;

function persist(session: DeliverySession) {
  window.localStorage.setItem(
    session.key,
    JSON.stringify(
      [...session.pending.values()].map((entry) => entry.notification),
    ),
  );
}

function restore(key: string): DeliverySession["pending"] {
  const raw = window.localStorage.getItem(key);
  if (!raw) return new Map();
  const records: unknown = JSON.parse(raw);
  if (!Array.isArray(records))
    throw new Error("Invalid live notification retry journal");
  const pending: DeliverySession["pending"] = new Map();
  for (const record of records.slice(-MAX_PENDING)) {
    if (
      typeof record?.id !== "string" ||
      !["dm", "thread_reply"].includes(record.slot) ||
      typeof record.payload?.title !== "string"
    ) {
      throw new Error("Invalid live notification retry entry");
    }
    pending.set(record.id, { notification: record, attempts: 0, due: 0 });
  }
  return pending;
}

/**
 * Persists up to 500 alerts per community/viewer, evicting the oldest on overflow.
 * Failed alerts retry five times per mount; exhausted entries stay durable for
 * the next mount. Delivery is at-least-once if the app exits before persisting
 * a successful native send. Settings changes pause delivery, not the journal.
 */
export function useLiveNotificationDelivery({
  scope,
  enabled,
  dmEnabled,
  threadReplyEnabled,
  onDelivered,
}: {
  scope: string | null;
  enabled: boolean;
  dmEnabled: boolean;
  threadReplyEnabled: boolean;
  onDelivered: (notification: LiveNotification) => void;
}) {
  const sessionRef = React.useRef<DeliverySession | null>(null);

  const canSend = React.useEffectEvent(
    (session: DeliverySession, notification: LiveNotification) =>
      !session.disposed &&
      sessionRef.current === session &&
      enabled &&
      (notification.slot === "dm" ? dmEnabled : threadReplyEnabled),
  );
  const delivered = React.useEffectEvent(onDelivered);

  const flush = React.useEffectEvent(async (session: DeliverySession) => {
    if (session.disposed || session.running) return;
    clearTimeout(session.timer);
    const entry = [...session.pending.values()]
      .filter(
        (candidate) =>
          candidate.attempts < MAX_ATTEMPTS &&
          canSend(session, candidate.notification),
      )
      .sort((left, right) => left.due - right.due)[0];
    if (!entry) return;
    const delay = entry.due - Date.now();
    if (delay > 0) {
      session.timer = setTimeout(() => {
        void flush(session);
      }, delay);
      return;
    }
    session.running = true;
    const generation = session.generation;
    try {
      const permitted = await ensureDesktopNotificationPermissionGranted();
      if (
        generation !== session.generation ||
        !canSend(session, entry.notification)
      )
        return;
      const sent =
        permitted &&
        (await sendDesktopNotification(
          entry.notification.payload,
          () =>
            generation === session.generation &&
            canSend(session, entry.notification),
        ));
      if (session.disposed) return;
      if (sent) {
        session.pending.delete(entry.notification.id);
        persist(session);
        if (
          generation === session.generation &&
          canSend(session, entry.notification)
        ) {
          delivered(entry.notification);
        }
      } else {
        entry.attempts += 1;
        entry.due =
          Date.now() + Math.min(1_000 * 2 ** (entry.attempts - 1), 30_000);
      }
    } catch (error) {
      entry.attempts = MAX_ATTEMPTS;
      console.error(
        "Live notification delivery failed; retry journal retained",
        error,
      );
    } finally {
      session.running = false;
      if (!session.disposed)
        session.timer = setTimeout(() => {
          void flush(session);
        }, 0);
    }
  });

  React.useEffect(() => {
    if (!scope) return;
    const key = `buzz-live-notification-retry.v1:${scope}`;
    const session: DeliverySession = {
      key,
      pending: restore(key),
      running: false,
      disposed: false,
      generation: 0,
    };
    sessionRef.current = session;
    void flush(session);
    return () => {
      session.disposed = true;
      session.generation += 1;
      clearTimeout(session.timer);
      if (sessionRef.current === session) sessionRef.current = null;
    };
  }, [scope]);

  React.useEffect(() => {
    if (!scope || !enabled || (!dmEnabled && !threadReplyEnabled)) return;
    const session = sessionRef.current;
    if (!session) return;
    void flush(session);
    return () => {
      session.generation += 1;
      clearTimeout(session.timer);
    };
  }, [scope, enabled, dmEnabled, threadReplyEnabled]);

  return React.useEffectEvent((notification: LiveNotification) => {
    const session = sessionRef.current;
    if (
      !session ||
      !canSend(session, notification) ||
      session.pending.has(notification.id)
    )
      return;
    session.pending.set(notification.id, { notification, attempts: 0, due: 0 });
    for (const id of session.pending.keys()) {
      if (session.pending.size <= MAX_PENDING) break;
      session.pending.delete(id);
    }
    persist(session);
    void flush(session);
  });
}
