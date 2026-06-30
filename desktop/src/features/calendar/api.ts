import { invokeTauri } from "@/shared/api/tauri";

export type GoogleCalendarStatus = {
  configured: boolean;
  connected: boolean;
  connectedAt: number | null;
  scopes: string[];
};

export type GoogleCalendarEvent = {
  id: string;
  title: string;
  startsAt: string;
  endsAt: string;
  allDay: boolean;
  joinUrl: string | null;
  htmlUrl: string | null;
  transparency: string | null;
};

type RawGoogleCalendarStatus = {
  configured: boolean;
  connected: boolean;
  connected_at: number | null;
  scopes: string[];
};

type RawGoogleCalendarEvent = {
  id: string;
  title: string;
  starts_at: string;
  ends_at: string;
  all_day: boolean;
  join_url: string | null;
  html_url: string | null;
  transparency: string | null;
};

function fromRawStatus(raw: RawGoogleCalendarStatus): GoogleCalendarStatus {
  return {
    configured: raw.configured,
    connected: raw.connected,
    connectedAt: raw.connected_at,
    scopes: raw.scopes,
  };
}

function fromRawEvent(raw: RawGoogleCalendarEvent): GoogleCalendarEvent {
  return {
    id: raw.id,
    title: raw.title,
    startsAt: raw.starts_at,
    endsAt: raw.ends_at,
    allDay: raw.all_day,
    joinUrl: raw.join_url,
    htmlUrl: raw.html_url,
    transparency: raw.transparency,
  };
}

export async function getGoogleCalendarStatus(): Promise<GoogleCalendarStatus> {
  return fromRawStatus(
    await invokeTauri<RawGoogleCalendarStatus>("get_google_calendar_status"),
  );
}

export async function connectGoogleCalendar(): Promise<GoogleCalendarStatus> {
  return fromRawStatus(
    await invokeTauri<RawGoogleCalendarStatus>("connect_google_calendar"),
  );
}

export async function disconnectGoogleCalendar(): Promise<GoogleCalendarStatus> {
  return fromRawStatus(
    await invokeTauri<RawGoogleCalendarStatus>("disconnect_google_calendar"),
  );
}

export async function getGoogleCalendarEvents(input: {
  timeMax: string;
  timeMin: string;
}): Promise<GoogleCalendarEvent[]> {
  const raw = await invokeTauri<RawGoogleCalendarEvent[]>(
    "get_google_calendar_events",
    input,
  );
  return raw.map(fromRawEvent);
}
