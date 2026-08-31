import type { RelayEvent } from "@/shared/api/types";
import { fetchHostHistory, type HostHistoryFilter } from "./history";
import {
  canonicalHostEvent,
  type HostPublicationJournal,
} from "./pendingPublication";
import { validateHostReport } from "./reportValidation";

export const HOST_KIND = 50000;
export const HOST_NAMESPACE = "buzz.host.v1";
export type HostReport = {
  v: number;
  name: string;
  os: string;
  arch: string;
  launcher_version: string;
  runtimes: {
    id: string;
    label: string;
    availability: string;
    auth_status: string;
  }[];
  accepts_start: boolean;
  provisioned?: { agent: string; runtime: string; revision: string }[];
};
export type LocalHost = { host: string; report: HostReport };
export type HostRow = {
  host: string;
  registration: RelayEvent;
  event?: RelayEvent;
  report?: HostReport;
};
export type HostSnapshot = {
  rows: HostRow[];
  local?: LocalHost;
  error?: string;
  checking: boolean;
};
export type HostBridge = {
  local(): Promise<LocalHost>;
  registration(): Promise<RelayEvent>;
  report(registration: RelayEvent): Promise<RelayEvent>;
  inspect(registration: RelayEvent): Promise<string>;
  decode(registration: RelayEvent, report: RelayEvent): Promise<HostReport>;
};
export type HostRelay = {
  fetchEvents(filter: HostHistoryFilter): Promise<RelayEvent[]>;
  publishEvent(event: RelayEvent): Promise<void>;
};
export const hostQueryKey = (
  relay: string | undefined,
  owner: string | undefined,
) => ["hosts", relay, owner] as const;
export function tag(event: RelayEvent, name: string) {
  return event.tags.find((t) => t[0] === name)?.[1];
}
function sameSignedEvent(a: RelayEvent, b: RelayEvent) {
  try {
    return (
      JSON.stringify(canonicalHostEvent(a)) ===
      JSON.stringify(canonicalHostEvent(b))
    );
  } catch {
    return false;
  }
}
function newest(events: RelayEvent[]) {
  return [...events].sort((a, b) => {
    // Native inspection is authoritative; tolerate malformed transport objects
    // here so one invalid record cannot prevent inspecting the valid candidates.
    const timestamp = (e: RelayEvent) =>
      Number.isSafeInteger(e?.created_at) ? e.created_at : 0;
    const id = (e: RelayEvent) => (typeof e?.id === "string" ? e.id : "");
    return (
      timestamp(b) - timestamp(a) ||
      (id(a) < id(b) ? -1 : id(a) > id(b) ? 1 : 0)
    );
  });
}
export function isFresh(event: RelayEvent | undefined, now: number) {
  return (
    !!event &&
    event.created_at <= now + 30 &&
    Number(tag(event, "valid_until")) > now
  );
}
export function needsReport(
  previous: HostRow | undefined,
  current: HostReport,
  _now: number,
) {
  // Compare structured data, not randomized NIP-44 ciphertext or object key order.
  const canonical = (r: HostReport) =>
    JSON.stringify([
      r.v,
      r.name,
      r.os,
      r.arch,
      r.launcher_version,
      r.accepts_start,
      [...(r.provisioned ?? [])]
        .sort((a, b) => a.agent.localeCompare(b.agent))
        .map((c) => [c.agent, c.runtime, c.revision]),
      [...r.runtimes]
        .sort((a, b) => a.id.localeCompare(b.id))
        .map((x) => [x.id, x.label, x.availability, x.auth_status]),
    ]);
  return (
    !previous?.report ||
    !previous.event ||
    tag(previous.event, "l") !== "profile" ||
    canonical(previous.report) !== canonical(current)
  );
}

/** One serialized read-before-write pass. Failed reads never become empty state. */
export async function reconcileHost(args: {
  owner: string;
  relay: HostRelay;
  bridge: HostBridge;
  journal: HostPublicationJournal;
  active: () => boolean;
  now: () => number;
}): Promise<HostSnapshot> {
  const { owner, relay, bridge, journal, now } = args;
  const check = () => {
    if (!args.active()) throw new Error("Host registration cancelled");
  };
  const filter = (label: string): HostHistoryFilter => ({
    kinds: [HOST_KIND],
    "#p": [owner],
    "#L": [HOST_NAMESPACE],
    "#l": [label],
    limit: 1000,
  });
  check();
  const local = await bridge.local();
  check();
  validateHostReport(local.report);
  const pending = journal.load();
  let pendingReport: HostReport | undefined;
  if (pending) {
    // Disk is untrusted. Do not use cached plaintext or trust a stored host ID.
    if ((await bridge.inspect(pending.registration)) !== local.host)
      throw new Error("Pending publication belongs to a different local host");
    check();
    if (pending.report) {
      pendingReport = await bridge.decode(pending.registration, pending.report);
      check();
      validateHostReport(pendingReport);
    }
  }
  const registrations = await fetchHostHistory(
    (page) => relay.fetchEvents(page),
    { ...filter("registration"), authors: [owner] },
    check,
  );
  check();
  const rows = new Map<string, HostRow>();
  const bindings = new Map<
    string,
    { host: string; registration: RelayEvent }
  >();
  let unreadableRegistration = false;
  for (const registration of newest(registrations)) {
    let host: string;
    try {
      host = await bridge.inspect(registration);
    } catch {
      // A newer invalid record must not hide an older verified binding.
      // IPC errors are not typed: absence is unsafe if no local binding survives.
      unreadableRegistration = true;
      check();
      continue;
    }
    check();
    bindings.set(registration.id, { host, registration });
    if (!rows.has(host)) rows.set(host, { host, registration });
  }
  if (unreadableRegistration && !rows.has(local.host))
    throw new Error("Cannot establish local host registration from history");
  const pendingRegistrationKnown =
    pending && bindings.has(pending.registration.id);
  if (pending?.report && !pendingRegistrationKnown)
    throw new Error("Pending report registration is missing from history");
  if (pending && !pending.report && !pendingRegistrationKnown) {
    // Include the pending host in the read set, but do not return/promote this
    // row until the exact registration has received an accepted ACK below.
    bindings.set(pending.registration.id, {
      host: local.host,
      registration: pending.registration,
    });
    if (!rows.has(local.host))
      rows.set(local.host, {
        host: local.host,
        registration: pending.registration,
      });
  }
  let pendingReportKnown = false;
  // Complete every history read before constructing or publishing anything.
  // In particular, an unreadable remembered host must not cause a new local
  // registration to be appended on an otherwise failed reconciliation pass.
  for (const row of rows.values()) {
    const events = await fetchHostHistory(
      (page) => relay.fetchEvents(page),
      {
        ...filter("report"),
        "#l": ["report", "profile"],
        authors: [row.host],
        "#x": [row.host],
      },
      check,
    );
    check();
    if (row.host === local.host && pending?.report)
      pendingReportKnown = events.some(
        (event) => pending.report && sameSignedEvent(event, pending.report),
      );
    for (const event of newest(events)) {
      let report: HostReport;
      try {
        // Duplicate durable bindings can exist from older clients. Reuse the
        // latest valid report for this owner+host even if it references an older
        // binding; a newer registration must not force duplicate capabilities.
        const binding = bindings.get(tag(event, "e") ?? "");
        if (!binding || binding.host !== row.host)
          throw new Error("Unknown host report binding");
        report = await bridge.decode(binding.registration, event);
        validateHostReport(report);
      } catch {
        check();
        continue;
      }
      check();
      row.event = event;
      row.report = report;
      break;
    }
    // An empty completed history is known absence; unreadable history is not.
    if (events.length && !row.report)
      throw new Error("Cannot establish host capabilities from history");
  }
  let own = rows.get(local.host);
  if (pending) {
    const event = pending.report ?? pending.registration;
    if (!(pending.report ? pendingReportKnown : pendingRegistrationKnown)) {
      check();
      // The relay rejects expired reports, but an earlier ingest may still
      // commit. Do not abandon the ID or mint a replacement: wait for history.
      if (
        pending.report &&
        tag(pending.report, "l") === "report" &&
        Number(tag(pending.report, "valid_until")) <= now()
      )
        throw new Error(
          "Unconfirmed host report expired; waiting for relay history",
        );
      // A completed primary read can overtake an old EVENT's commit. Re-send
      // its EXACT signed fields, never a new randomized event. Relay event-ID
      // uniqueness does the rest, regardless of which attempt commits first.
      await relay.publishEvent(event);
      check();
    }
    journal.clear();
    if (
      pending.report &&
      pendingReport &&
      own &&
      (!own.event ||
        newest([own.event, pending.report])[0].id === pending.report.id)
    ) {
      own.event = pending.report;
      own.report = pendingReport;
    }
    // Legacy reports are recovered by exact ID before upgrading to a durable profile.
  }
  if (!own) {
    const registration = canonicalHostEvent(await bridge.registration());
    check();
    if ((await bridge.inspect(registration)) !== local.host)
      throw new Error("Local host identity changed");
    check();
    journal.save({ v: 1, registration });
    check();
    await relay.publishEvent(registration);
    check();
    journal.clear();
    own = { host: local.host, registration };
    rows.set(local.host, own);
  }
  if (needsReport(own, local.report, now())) {
    // Kind 50000 uses (timestamp DESC, id ASC). Two changed reports in one
    // second could select the old payload forever. Retry at the next tick
    // rather than mint randomized ciphertext repeatedly at a tied timestamp.
    if (own.event && own.event.created_at >= now())
      throw new Error("Host changed within the current second; retry shortly");
    const event = canonicalHostEvent(await bridge.report(own.registration));
    check();
    const report = await bridge.decode(own.registration, event);
    check();
    validateHostReport(report);
    journal.save({ v: 1, registration: own.registration, report: event });
    check();
    await relay.publishEvent(event);
    check();
    journal.clear();
    own.event = event;
    own.report = report;
  }
  return { rows: [...rows.values()], local, checking: false };
}
