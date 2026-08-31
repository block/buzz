import { reconcileHost, HOST_KIND, HOST_NAMESPACE } from "./registration.ts";

import { createHostPublicationJournal } from "./pendingPublication.ts";

export function memoryStorage() {
  const entries = new Map();
  return {
    entries,
    getItem: (key) => entries.get(key) ?? null,
    setItem: (key, value) => entries.set(key, value),
    removeItem: (key) => entries.delete(key),
  };
}

export function fixture({ legacy = false } = {}) {
  let now = 100;
  const owner = "a".repeat(64),
    host = "b".repeat(64);
  const payload = {
    v: legacy ? 1 : 2,
    name: "computer",
    os: "macos",
    arch: "aarch64",
    launcher_version: "test",
    runtimes: [
      {
        id: "one",
        label: "One",
        availability: "available",
        auth_status: "unknown",
      },
    ],
    accepts_start: false,
  };
  const storage = memoryStorage();
  const journal = createHostPublicationJournal(
    "wss://fixture.invalid",
    owner,
    storage,
  );
  const decoded = new Map();
  const events = [];
  const writes = [];
  let count = 0;
  let active = true;
  const make = (label, extra = []) => ({
    id: String(now * 1000 + ++count).padStart(64, "0"),
    kind: HOST_KIND,
    pubkey: label === "registration" ? owner : host,
    content: "random-ciphertext",
    sig: "f".repeat(128),
    created_at: now,
    tags: [
      ["L", HOST_NAMESPACE],
      ["l", label, HOST_NAMESPACE],
      ["p", owner],
      ["x", host],
      ...extra,
    ],
  });
  const bridge = {
    local: async () => ({ host, report: payload }),
    registration: async () => make("registration"),
    report: async (registration) => {
      const event = {
        ...make(legacy ? "report" : "profile", [
          ["e", registration.id],
          ...(legacy ? [["valid_until", String(now + 180)]] : []),
        ]),
        decoded: structuredClone(payload),
      };
      decoded.set(event.id, event.decoded);
      return event;
    },
    inspect: async (registration) => {
      if (registration.pubkey !== owner)
        throw new Error("foreign registration");
      return host;
    },
    decode: async (_registration, report) =>
      report.decoded ?? decoded.get(report.id),
  };
  const relay = {
    fetchEvents: async (filter) =>
      events
        .filter(
          (e) =>
            (!filter.authors || filter.authors.includes(e.pubkey)) &&
            (filter.until === undefined ||
              e.created_at < filter.until ||
              (e.created_at === filter.until &&
                (!filter.before_id || e.id > filter.before_id))) &&
            Object.entries(filter)
              .filter(([k]) => k.startsWith("#"))
              .every(([k, values]) =>
                e.tags.some(
                  (t) => t[0] === k.slice(1) && values.includes(t[1]),
                ),
              ),
        )
        .sort((a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id))
        .slice(0, filter.limit),
    publishEvent: async (event) => {
      writes.push(event);
      if (!events.some((stored) => stored.id === event.id)) events.push(event);
    },
  };
  const run = (overrides = {}) =>
    reconcileHost({
      owner,
      relay,
      bridge,
      journal,
      active: () => active,
      now: () => now,
      ...overrides,
    });
  return {
    run,
    journal,
    storage,
    decoded,
    relay,
    bridge,
    writes,
    events,
    payload,
    setNow: (t) => {
      now = t;
    },
    stop: () => {
      active = false;
    },
  };
}
