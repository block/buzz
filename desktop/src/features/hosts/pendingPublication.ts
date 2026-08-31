import type { RelayEvent } from "@/shared/api/types";

/** Only canonical signed, encrypted events; never decoded host metadata or keys. */
export type PendingHostPublication = {
  v: 1;
  registration: RelayEvent;
  report?: RelayEvent;
};
export type HostPublicationJournal = {
  load(): PendingHostPublication | undefined;
  save(pending: PendingHostPublication): void;
  clear(): void;
};

const FIELDS = ["id", "pubkey", "kind", "content", "created_at", "tags", "sig"];
const HEX = /^[0-9a-f]{64}$/;
const MAX_BYTES = 1024 * 1024;
const invalid = () =>
  new Error("Host pending publication is unavailable or invalid");

/** Strip transport/native adornments before saving or sending a signed event. */
export function canonicalHostEvent(event: RelayEvent): RelayEvent {
  return {
    id: event.id,
    pubkey: event.pubkey,
    kind: event.kind,
    content: event.content,
    created_at: event.created_at,
    tags: event.tags.map((tag) => [...tag]),
    sig: event.sig,
  };
}

function eventShape(value: unknown): value is RelayEvent {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const e = value as RelayEvent;
  return (
    Object.keys(e).length === FIELDS.length &&
    Object.keys(e).every((key) => FIELDS.includes(key)) &&
    typeof e.id === "string" &&
    HEX.test(e.id) &&
    typeof e.pubkey === "string" &&
    HEX.test(e.pubkey) &&
    e.kind === 50000 &&
    typeof e.content === "string" &&
    e.content.length > 0 &&
    Number.isSafeInteger(e.created_at) &&
    e.created_at >= 0 &&
    typeof e.sig === "string" &&
    /^[0-9a-f]{128}$/.test(e.sig) &&
    Array.isArray(e.tags) &&
    e.tags.every(
      (tag) => Array.isArray(tag) && tag.every((s) => typeof s === "string"),
    )
  );
}

function parse(raw: string): PendingHostPublication {
  if (raw.length * 2 > MAX_BYTES) throw invalid();
  const value = JSON.parse(raw);
  if (
    value?.v !== 1 ||
    Object.keys(value).some(
      (key) => !["v", "registration", "report"].includes(key),
    ) ||
    !eventShape(value.registration) ||
    ("report" in value && !eventShape(value.report))
  )
    throw invalid();
  return value;
}

/**
 * Load-bearing localStorage journal, scoped by exact relay URL and signer.
 * Storage/corruption errors fail closed, never become absence. Unlike caches it
 * must not be evicted or fall back to memory on quota failure. Native inspection
 * and decryption remain authoritative: shape checks are NOT signature checks.
 */
export function createHostPublicationJournal(
  relay: string,
  owner: string,
  storage?: Pick<Storage, "getItem" | "setItem" | "removeItem">,
): HostPublicationJournal {
  const getStorage = () => storage ?? localStorage;
  const key = `buzz-host-pending.v1:${JSON.stringify([relay, owner])}`;
  return {
    load() {
      try {
        const raw = getStorage().getItem(key);
        return raw === null ? undefined : parse(raw);
      } catch {
        throw invalid();
      }
    },
    save(pending) {
      try {
        // Refuse overwriting an unresolved or malformed attempt.
        if (getStorage().getItem(key) !== null) throw invalid();
        const raw = JSON.stringify({
          v: 1,
          registration: canonicalHostEvent(pending.registration),
          ...(pending.report
            ? { report: canonicalHostEvent(pending.report) }
            : {}),
        });
        parse(raw);
        getStorage().setItem(key, raw);
        if (getStorage().getItem(key) !== raw) throw invalid();
      } catch {
        // Never include diagnostics which could echo payloads or private paths.
        throw invalid();
      }
    },
    clear() {
      try {
        getStorage().removeItem(key);
        if (getStorage().getItem(key) !== null) throw invalid();
      } catch {
        throw invalid();
      }
    },
  };
}
