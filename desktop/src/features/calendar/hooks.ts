import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  connectGoogleCalendar,
  disconnectGoogleCalendar,
  getGoogleCalendarEvents,
  getGoogleCalendarStatus,
  type GoogleCalendarEvent,
} from "@/features/calendar/api";

export const googleCalendarStatusQueryKey = ["google-calendar-status"] as const;

export const googleCalendarEventsQueryKey = (
  timeMin: string,
  timeMax: string,
) => ["google-calendar-events", timeMin, timeMax] as const;

const calendarBusyTimeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: "numeric",
  minute: "2-digit",
});

function useCalendarNow(enabled: boolean): number {
  const [now, setNow] = React.useState(() => Date.now());

  React.useEffect(() => {
    if (!enabled) return;

    setNow(Date.now());
    const interval = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(interval);
  }, [enabled]);

  return now;
}

export function useGoogleCalendarStatusQuery({
  enabled = true,
}: {
  enabled?: boolean;
} = {}) {
  return useQuery({
    enabled,
    queryKey: googleCalendarStatusQueryKey,
    queryFn: getGoogleCalendarStatus,
    staleTime: 60_000,
  });
}

export function useGoogleCalendarEventsQuery({
  enabled,
  timeMax,
  timeMin,
}: {
  enabled: boolean;
  timeMax: string;
  timeMin: string;
}) {
  return useQuery({
    enabled,
    queryKey: googleCalendarEventsQueryKey(timeMin, timeMax),
    queryFn: () => getGoogleCalendarEvents({ timeMax, timeMin }),
    refetchInterval: enabled ? 60_000 : false,
    staleTime: 45_000,
  });
}

export function useGoogleCalendarConnectionMutations() {
  const queryClient = useQueryClient();
  const invalidate = () => {
    void queryClient.invalidateQueries({
      queryKey: googleCalendarStatusQueryKey,
    });
    void queryClient.invalidateQueries({
      predicate: (query) => query.queryKey[0] === "google-calendar-events",
    });
  };

  const connect = useMutation({
    mutationFn: connectGoogleCalendar,
    onSuccess: invalidate,
  });
  const disconnect = useMutation({
    mutationFn: disconnectGoogleCalendar,
    onSuccess: invalidate,
  });

  return { connect, disconnect };
}

export function eventStartDate(event: GoogleCalendarEvent): Date {
  return parseCalendarDate(event.startsAt, event.allDay);
}

export function eventEndDate(event: GoogleCalendarEvent): Date {
  return parseCalendarDate(event.endsAt, event.allDay);
}

export function isBusyCalendarEvent(event: GoogleCalendarEvent): boolean {
  return !event.allDay && event.transparency !== "transparent";
}

export function isOngoingCalendarEvent(
  event: GoogleCalendarEvent,
  nowMs: number,
): boolean {
  return (
    isBusyCalendarEvent(event) &&
    eventStartDate(event).getTime() <= nowMs &&
    eventEndDate(event).getTime() > nowMs
  );
}

export function isUpcomingCalendarEvent(
  event: GoogleCalendarEvent,
  nowMs: number,
): boolean {
  return isBusyCalendarEvent(event) && eventStartDate(event).getTime() > nowMs;
}

export function formatCalendarBusyLabel(event: GoogleCalendarEvent): string {
  return `In meeting until ${calendarBusyTimeFormatter.format(
    eventEndDate(event),
  )}`;
}

export function parseCalendarDate(value: string, allDay: boolean): Date {
  if (allDay && /^\d{4}-\d{2}-\d{2}$/.test(value)) {
    return new Date(`${value}T00:00:00`);
  }
  return new Date(value);
}

export function todayCalendarWindow(nowMs = Date.now()): {
  timeMax: string;
  timeMin: string;
} {
  const start = new Date(nowMs);
  start.setHours(0, 0, 0, 0);
  const end = new Date(start);
  end.setDate(end.getDate() + 1);
  return {
    timeMin: start.toISOString(),
    timeMax: end.toISOString(),
  };
}

export function currentCalendarWindow(nowMs = Date.now()): {
  timeMax: string;
  timeMin: string;
} {
  const start = new Date(nowMs - 60 * 60 * 1_000);
  const end = new Date(nowMs + 8 * 60 * 60 * 1_000);
  return {
    timeMin: start.toISOString(),
    timeMax: end.toISOString(),
  };
}

export function useCurrentGoogleCalendarEvent({
  enabled = true,
}: {
  enabled?: boolean;
} = {}) {
  const now = useCalendarNow(enabled);
  const statusQuery = useGoogleCalendarStatusQuery({ enabled });
  const window = React.useMemo(() => todayCalendarWindow(now), [now]);
  const eventsQuery = useGoogleCalendarEventsQuery({
    enabled: Boolean(
      enabled && statusQuery.data?.configured && statusQuery.data.connected,
    ),
    timeMax: window.timeMax,
    timeMin: window.timeMin,
  });
  const events = eventsQuery.data ?? [];
  const sortedEvents = React.useMemo(
    () =>
      [...events].sort(
        (a, b) => eventStartDate(a).getTime() - eventStartDate(b).getTime(),
      ),
    [events],
  );
  const currentEvent = React.useMemo(
    () =>
      sortedEvents.find((event) => isOngoingCalendarEvent(event, now)) ?? null,
    [sortedEvents, now],
  );
  const nextEvent = React.useMemo(
    () =>
      sortedEvents.find((event) => isUpcomingCalendarEvent(event, now)) ?? null,
    [sortedEvents, now],
  );

  return {
    currentEvent,
    eventsQuery,
    isConnected: Boolean(statusQuery.data?.connected),
    isConfigured: Boolean(statusQuery.data?.configured),
    nextEvent,
    statusQuery,
  };
}
