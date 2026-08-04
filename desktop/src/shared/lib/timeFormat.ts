import * as React from "react";

/**
 * User preference for the clock style used everywhere the app prints a time of
 * day — message rows, inbox labels, agent transcripts, project panels, search
 * results.
 *
 * - `system` — follow the host's clock convention (what the OS locale uses).
 * - `12-hour` — 12-hour clock with an AM/PM marker ("2:34 PM").
 * - `24-hour` — 24-hour clock ("14:34").
 *
 * Persisted in localStorage. This is a device-level UI preference, not
 * community-scoped data, so it is intentionally not reset on community switch.
 *
 * Defaults to `12-hour` rather than `system`: every clock in the app was
 * hardcoded to `en-US` before this preference existed, so 12-hour is what
 * existing installs already show. Keeping it as the default means upgrading
 * changes nothing until the user picks a clock, and keeps rendered times
 * deterministic in tests and screenshots regardless of host locale.
 *
 * Date wording stays as-is: the app pins `en-US` for month/weekday names, and
 * this preference only overrides the hour cycle, so switching to 24-hour does
 * not silently re-localize every date label.
 */
export type TimeFormatPreference = "system" | "12-hour" | "24-hour";

const STORAGE_KEY = "buzz.appearance.timeFormat";

/** Clock used when nothing is stored, or the stored value is unrecognized. */
const DEFAULT_TIME_FORMAT_PREFERENCE: TimeFormatPreference = "12-hour";

const listeners = new Set<() => void>();

let timeFormatPreference = readStoredPreference();

function parseTimeFormatPreference(
  value: string | null | undefined,
): TimeFormatPreference {
  return value === "system" || value === "12-hour" || value === "24-hour"
    ? value
    : DEFAULT_TIME_FORMAT_PREFERENCE;
}

function readStoredPreference(): TimeFormatPreference {
  try {
    return parseTimeFormatPreference(
      globalThis.localStorage?.getItem(STORAGE_KEY),
    );
  } catch {
    return DEFAULT_TIME_FORMAT_PREFERENCE;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): TimeFormatPreference {
  return timeFormatPreference;
}

function getServerSnapshot(): TimeFormatPreference {
  return DEFAULT_TIME_FORMAT_PREFERENCE;
}

/** Read the persisted clock preference outside of React. */
export function getTimeFormatPreference(): TimeFormatPreference {
  return timeFormatPreference;
}

/** Update the clock preference and notify all subscribed components. */
export function setTimeFormatPreference(
  preference: TimeFormatPreference,
): void {
  timeFormatPreference = preference;

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, preference);
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/**
 * The active clock preference. Components that render a time — or that memoize
 * a formatted time string — read this so a change in Settings repaints them
 * immediately instead of waiting for the next unrelated data update.
 */
export function useTimeFormatPreference(): TimeFormatPreference {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

let cachedSystemHour12: boolean | null = null;

/**
 * Whether the host's own locale prints a 12-hour clock. Resolved from `Intl`
 * rather than the app's pinned `en-US`, so `system` tracks the OS convention
 * (a Swedish or German host reads as 24-hour) without changing date wording.
 */
function systemPrefersHour12(): boolean {
  if (cachedSystemHour12 !== null) return cachedSystemHour12;

  try {
    const resolved = new Intl.DateTimeFormat(undefined, {
      hour: "numeric",
    }).resolvedOptions();
    cachedSystemHour12 =
      resolved.hour12 ??
      (resolved.hourCycle === "h11" || resolved.hourCycle === "h12");
  } catch {
    cachedSystemHour12 = true;
  }

  return cachedSystemHour12;
}

/** Resolves `system` against the host locale; `12-hour`/`24-hour` are literal. */
export function resolvesToHour12(
  preference: TimeFormatPreference = getTimeFormatPreference(),
): boolean {
  if (preference === "12-hour") return true;
  if (preference === "24-hour") return false;
  return systemPrefersHour12();
}

/**
 * Overlays the resolved hour cycle onto `Intl.DateTimeFormat` options.
 *
 * 24-hour uses an explicit `h23` cycle so midnight prints `00:xx` rather than
 * the `h24` (`24:xx`) some locale defaults would pick. Whichever key is unused
 * is set to `undefined`, which `Intl` treats as absent — that also clears any
 * `hour12` the caller had hardcoded.
 */
export function withClockOptions(
  options: Intl.DateTimeFormatOptions,
  hour12: boolean = resolvesToHour12(),
): Intl.DateTimeFormatOptions {
  return {
    ...options,
    hour12: hour12 ? true : undefined,
    hourCycle: hour12 ? undefined : "h23",
  };
}

/**
 * Builds a formatter that honors the current clock preference.
 *
 * Drop-in replacement for a module-level `new Intl.DateTimeFormat(...)` whose
 * `.format(date)` is called later: the underlying formatter is built once and
 * rebuilt only when the preference flips, so hot paths (a channel timeline
 * formatting every visible row) keep the cached instance.
 */
export function createClockFormatter(
  locales: string | string[] | undefined,
  options: Intl.DateTimeFormatOptions,
): (date: Date) => string {
  let formatter: Intl.DateTimeFormat | null = null;
  let formatterHour12: boolean | null = null;

  return (date: Date) => {
    const hour12 = resolvesToHour12();

    if (!formatter || formatterHour12 !== hour12) {
      formatterHour12 = hour12;
      formatter = new Intl.DateTimeFormat(
        locales,
        withClockOptions(options, hour12),
      );
    }

    return formatter.format(date);
  };
}
