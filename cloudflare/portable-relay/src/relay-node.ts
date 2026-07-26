import { DurableObject } from "cloudflare:workers";
import {
  matchFilter,
  matchFilters,
  sortEvents,
  verifyEvent,
  type Event,
  type Filter,
} from "nostr-tools";
import {
  eventFromUnknown,
  filtersFromUnknown,
  ProtocolInputError,
} from "./protocol";

const DEFAULT_QUERY_LIMIT = 500;
const MAX_QUERY_LIMIT = 5_000;
const MAX_SUBSCRIPTIONS_PER_CONNECTION = 128;
const MAX_SUBSCRIPTION_ID_LENGTH = 128;

export const STABLE_NODE_KEY_HEADER = "X-Buzz-Stable-Node-Key";

export interface RelayNodeDescription {
  stableNodeKey: string;
}

export interface WriteResult {
  event_id: string;
  accepted: boolean;
  message: "stored" | "duplicate" | "superseded" | "ephemeral" | "invalid";
}

interface RelayNodeRow extends Record<string, SqlStorageValue> {
  stable_node_key: string;
}

interface AcceptedEventRow extends Record<string, SqlStorageValue> {
  decision: string;
}

interface EffectiveEventRow extends Record<string, SqlStorageValue> {
  event_json: string;
}

interface SubscriptionRow extends Record<string, SqlStorageValue> {
  filters_json: string;
  subscription_id: string;
}

interface ConnectionAttachment {
  version: 1;
  connectionId: string;
}

/**
 * Durable state boundary for one normalized portable relay node.
 */
export class RelayNode extends DurableObject<Env> {
  readonly #sql: SqlStorage;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.#sql = ctx.storage.sql;
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS relay_node_metadata (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        stable_node_key TEXT NOT NULL
      ) STRICT
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS accepted_event_ids (
        event_id TEXT PRIMARY KEY,
        decision TEXT NOT NULL CHECK (decision IN ('stored', 'superseded'))
      ) WITHOUT ROWID, STRICT
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS event_journal (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        event_id TEXT NOT NULL UNIQUE,
        event_json TEXT NOT NULL,
        replacement_key TEXT,
        effective INTEGER NOT NULL CHECK (effective IN (0, 1))
      ) STRICT
    `);
    this.#sql.exec(`
      CREATE UNIQUE INDEX IF NOT EXISTS one_effective_replacement
      ON event_journal (replacement_key)
      WHERE replacement_key IS NOT NULL AND effective = 1
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS subscriptions (
        connection_id TEXT NOT NULL,
        subscription_id TEXT NOT NULL,
        filters_json TEXT NOT NULL,
        PRIMARY KEY (connection_id, subscription_id)
      ) WITHOUT ROWID, STRICT
    `);
  }

  /**
   * Binds this object permanently to the stable node key used to select it.
   */
  initializeNode(stableNodeKey: string): RelayNodeDescription {
    this.#sql.exec(
      `INSERT OR IGNORE INTO relay_node_metadata (singleton, stable_node_key)
       VALUES (1, ?)`,
      stableNodeKey,
    );

    const row = this.#sql
      .exec<RelayNodeRow>(
        "SELECT stable_node_key FROM relay_node_metadata WHERE singleton = 1",
      )
      .one();

    if (row.stable_node_key !== stableNodeKey) {
      throw new Error("durable object stable node key mismatch");
    }
    return { stableNodeKey: row.stable_node_key };
  }

  /**
   * Reports the durable node binding without exposing mutable relay state.
   */
  describeNode(): RelayNodeDescription | null {
    const rows = Array.from(
      this.#sql.exec<RelayNodeRow>(
        "SELECT stable_node_key FROM relay_node_metadata WHERE singleton = 1",
      ),
    );
    const row = rows[0];
    return row === undefined ? null : { stableNodeKey: row.stable_node_key };
  }

  /**
   * Verifies and applies one portable signed event behind SQLite's output gate.
   */
  submitEvent(stableNodeKey: string, event: Event): WriteResult {
    this.initializeNode(stableNodeKey);
    return this.#applyEvent(event);
  }

  /**
   * Returns the effective event set selected by NIP-01 filters.
   */
  queryEvents(stableNodeKey: string, filters: Filter[]): Event[] {
    this.initializeNode(stableNodeKey);
    return this.#queryEffective(filters);
  }

  /**
   * Counts effective events matching any supplied NIP-01 filter.
   */
  countEvents(stableNodeKey: string, filters: Filter[]): number {
    this.initializeNode(stableNodeKey);
    return this.#countEffective(filters);
  }

  #applyEvent(event: Event): WriteResult {
    if (!verifySafely(event)) {
      return {
        event_id: event.id,
        accepted: false,
        message: "invalid",
      };
    }

    const accepted = Array.from(
      this.#sql.exec<AcceptedEventRow>(
        "SELECT decision FROM accepted_event_ids WHERE event_id = ?",
        event.id,
      ),
    )[0];
    if (accepted !== undefined) {
      return {
        event_id: event.id,
        accepted: true,
        message: "duplicate",
      };
    }

    if (isEphemeralKind(event.kind)) {
      this.#publishLive(event);
      return {
        event_id: event.id,
        accepted: true,
        message: "ephemeral",
      };
    }

    const candidateKey = replacementKey(event);
    if (candidateKey !== null) {
      const current = this.#effectiveReplacement(candidateKey);
      if (current !== null && !replacementCandidateWins(event, current)) {
        this.ctx.storage.transactionSync(() => {
          this.#sql.exec(
            `INSERT INTO accepted_event_ids (event_id, decision)
             VALUES (?, 'superseded')`,
            event.id,
          );
        });
        return {
          event_id: event.id,
          accepted: true,
          message: "superseded",
        };
      }
    }

    const eventJson = JSON.stringify(event);
    this.ctx.storage.transactionSync(() => {
      this.#sql.exec(
        `INSERT INTO accepted_event_ids (event_id, decision)
         VALUES (?, 'stored')`,
        event.id,
      );
      if (candidateKey !== null) {
        this.#sql.exec(
          `UPDATE event_journal
           SET effective = 0
           WHERE replacement_key = ? AND effective = 1`,
          candidateKey,
        );
      }
      this.#sql.exec(
        `INSERT INTO event_journal (
           event_id, event_json, replacement_key, effective
         ) VALUES (?, ?, ?, 1)`,
        event.id,
        eventJson,
        candidateKey,
      );
    });
    this.#publishLive(event);

    return {
      event_id: event.id,
      accepted: true,
      message: "stored",
    };
  }

  #queryEffective(filters: Filter[]): Event[] {
    if (filters.length === 0) {
      return [];
    }

    const ordered = sortEvents(this.#effectiveEvents());
    const selected = new Map<string, Event>();
    for (const filter of filters) {
      const limit = Math.min(
        Math.max(filter.limit ?? DEFAULT_QUERY_LIMIT, 0),
        MAX_QUERY_LIMIT,
      );
      if (limit === 0) {
        continue;
      }
      let matched = 0;
      for (const event of ordered) {
        if (!matchFilter(filter, event)) {
          continue;
        }
        selected.set(event.id, event);
        matched += 1;
        if (matched >= limit) {
          break;
        }
      }
    }
    return sortEvents(Array.from(selected.values()));
  }

  #countEffective(filters: Filter[]): number {
    if (filters.length === 0) {
      return 0;
    }
    return this.#effectiveEvents().filter((event) =>
      matchFilters(filters, event),
    ).length;
  }

  /**
   * Accepts a hibernatable NIP-01 WebSocket for this durable node.
   */
  fetch(request: Request): Response {
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("Expected Upgrade: websocket", { status: 426 });
    }

    const stableNodeKey = request.headers.get(STABLE_NODE_KEY_HEADER);
    if (stableNodeKey === null || stableNodeKey === "") {
      return new Response("Missing stable node routing boundary", {
        status: 403,
      });
    }
    this.initializeNode(stableNodeKey);

    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment({
      version: 1,
      connectionId: crypto.randomUUID(),
    } satisfies ConnectionAttachment);
    return new Response(null, { status: 101, webSocket: client });
  }

  /**
   * Handles portable EVENT, REQ, and CLOSE frames after normal wake or hibernation.
   */
  webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): void {
    if (typeof message !== "string") {
      this.#send(ws, ["NOTICE", "binary messages are unsupported"]);
      return;
    }

    let frame: unknown;
    try {
      frame = JSON.parse(message);
    } catch {
      this.#send(ws, ["NOTICE", "invalid JSON"]);
      return;
    }
    if (!Array.isArray(frame) || typeof frame[0] !== "string") {
      this.#send(ws, ["NOTICE", "invalid relay frame"]);
      return;
    }

    if (frame[0] === "EVENT") {
      this.#handleWebSocketEvent(ws, frame);
      return;
    }
    if (frame[0] === "REQ") {
      this.#handleWebSocketReq(ws, frame);
      return;
    }
    if (frame[0] === "CLOSE") {
      this.#handleWebSocketClose(ws, frame);
      return;
    }
    this.#send(ws, ["NOTICE", "unsupported relay frame"]);
  }

  webSocketClose(ws: WebSocket): void {
    this.#deleteConnectionSubscriptions(ws);
  }

  webSocketError(ws: WebSocket, error: unknown): void {
    this.#deleteConnectionSubscriptions(ws);
    console.error("portable relay WebSocket error", {
      error: error instanceof Error ? error.name : "unknown_websocket_error",
    });
  }

  #effectiveEvents(): Event[] {
    return Array.from(
      this.#sql.exec<EffectiveEventRow>(
        "SELECT event_json FROM event_journal WHERE effective = 1",
      ),
      (row) => JSON.parse(row.event_json) as Event,
    );
  }

  #effectiveReplacement(key: string): Event | null {
    const row = Array.from(
      this.#sql.exec<EffectiveEventRow>(
        `SELECT event_json
         FROM event_journal
         WHERE replacement_key = ? AND effective = 1`,
        key,
      ),
    )[0];
    return row === undefined ? null : (JSON.parse(row.event_json) as Event);
  }

  #handleWebSocketEvent(ws: WebSocket, frame: unknown[]): void {
    try {
      const event = eventFromUnknown(frame[1]);
      const result = this.#applyEvent(event);
      this.#send(ws, ["OK", result.event_id, result.accepted, result.message]);
    } catch (error) {
      const message =
        error instanceof ProtocolInputError ? error.message : "invalid event";
      this.#send(ws, ["OK", eventIdFromFrame(frame), false, message]);
    }
  }

  #handleWebSocketReq(ws: WebSocket, frame: unknown[]): void {
    const subscriptionId = frame[1];
    if (
      typeof subscriptionId !== "string" ||
      subscriptionId.length === 0 ||
      subscriptionId.length > MAX_SUBSCRIPTION_ID_LENGTH
    ) {
      this.#send(ws, [
        "CLOSED",
        stringOrEmpty(subscriptionId),
        "invalid subscription ID",
      ]);
      return;
    }

    let filters: Filter[];
    try {
      filters = filtersFromUnknown(frame.slice(2));
      if (filters.length === 0) {
        throw new ProtocolInputError("REQ requires at least one filter");
      }
    } catch (error) {
      this.#send(ws, [
        "CLOSED",
        subscriptionId,
        error instanceof ProtocolInputError ? error.message : "invalid filters",
      ]);
      return;
    }

    const attachment = this.#attachmentOrNull(ws);
    if (attachment === null) {
      this.#send(ws, [
        "CLOSED",
        subscriptionId,
        "connection state unavailable",
      ]);
      return;
    }
    const connectionId = attachment.connectionId;
    const existing = this.#subscriptionCount(connectionId);
    const replacing = this.#hasSubscription(connectionId, subscriptionId);
    if (!replacing && existing >= MAX_SUBSCRIPTIONS_PER_CONNECTION) {
      this.#send(ws, ["CLOSED", subscriptionId, "too many subscriptions"]);
      return;
    }

    this.#sql.exec(
      `INSERT INTO subscriptions (
         connection_id, subscription_id, filters_json
       ) VALUES (?, ?, ?)
       ON CONFLICT (connection_id, subscription_id)
       DO UPDATE SET filters_json = excluded.filters_json`,
      connectionId,
      subscriptionId,
      JSON.stringify(filters),
    );

    for (const event of this.#queryEffective(filters)) {
      if (!this.#send(ws, ["EVENT", subscriptionId, event])) {
        return;
      }
    }
    this.#send(ws, ["EOSE", subscriptionId]);
  }

  #handleWebSocketClose(ws: WebSocket, frame: unknown[]): void {
    const subscriptionId = frame[1];
    if (typeof subscriptionId !== "string") {
      this.#send(ws, ["NOTICE", "invalid CLOSE"]);
      return;
    }
    const attachment = this.#attachmentOrNull(ws);
    if (attachment === null) {
      return;
    }
    this.#sql.exec(
      `DELETE FROM subscriptions
       WHERE connection_id = ? AND subscription_id = ?`,
      attachment.connectionId,
      subscriptionId,
    );
  }

  #publishLive(event: Event): void {
    for (const ws of this.ctx.getWebSockets()) {
      const attachment = this.#attachmentOrNull(ws);
      if (attachment === null) {
        continue;
      }
      const subscriptions = this.#sql.exec<SubscriptionRow>(
        `SELECT subscription_id, filters_json
         FROM subscriptions
         WHERE connection_id = ?`,
        attachment.connectionId,
      );
      for (const row of subscriptions) {
        const filters = JSON.parse(row.filters_json) as Filter[];
        if (
          matchFilters(filters, event) &&
          !this.#send(ws, ["EVENT", row.subscription_id, event])
        ) {
          // A dead socket must not block delivery to the remaining
          // connections or fail the already-committed submission.
          this.#sql.exec(
            "DELETE FROM subscriptions WHERE connection_id = ?",
            attachment.connectionId,
          );
          break;
        }
      }
    }
  }

  #deleteConnectionSubscriptions(ws: WebSocket): void {
    const attachment = this.#attachmentOrNull(ws);
    if (attachment === null) {
      return;
    }
    this.#sql.exec(
      "DELETE FROM subscriptions WHERE connection_id = ?",
      attachment.connectionId,
    );
  }

  #subscriptionCount(connectionId: string): number {
    const row = this.#sql
      .exec<{ count: number } & Record<string, SqlStorageValue>>(
        `SELECT COUNT(*) AS count
         FROM subscriptions
         WHERE connection_id = ?`,
        connectionId,
      )
      .one();
    return row.count;
  }

  #hasSubscription(connectionId: string, subscriptionId: string): boolean {
    return (
      Array.from(
        this.#sql.exec(
          `SELECT 1
           FROM subscriptions
           WHERE connection_id = ? AND subscription_id = ?`,
          connectionId,
          subscriptionId,
        ),
      ).length === 1
    );
  }

  #attachmentOrNull(ws: WebSocket): ConnectionAttachment | null {
    const attachment = ws.deserializeAttachment() as
      | ConnectionAttachment
      | undefined;
    if (
      attachment?.version !== 1 ||
      typeof attachment.connectionId !== "string"
    ) {
      return null;
    }
    return attachment;
  }

  #send(ws: WebSocket, frame: unknown[]): boolean {
    try {
      ws.send(JSON.stringify(frame));
      return true;
    } catch {
      return false;
    }
  }
}

function verifySafely(event: Event): boolean {
  try {
    return verifyEvent(event);
  } catch {
    return false;
  }
}

function isEphemeralKind(kind: number): boolean {
  return kind >= 20_000 && kind < 30_000;
}

function replacementKey(event: Event): string | null {
  if (
    event.kind === 0 ||
    event.kind === 3 ||
    (event.kind >= 10_000 && event.kind < 20_000)
  ) {
    return JSON.stringify(["replaceable", event.pubkey, event.kind]);
  }
  if (event.kind >= 30_000 && event.kind < 40_000) {
    const identifier = event.tags.find((tag) => tag[0] === "d")?.[1] ?? "";
    return JSON.stringify([
      "parameterized",
      event.pubkey,
      event.kind,
      identifier,
    ]);
  }
  return null;
}

function replacementCandidateWins(candidate: Event, current: Event): boolean {
  return (
    candidate.created_at > current.created_at ||
    (candidate.created_at === current.created_at && candidate.id < current.id)
  );
}

function eventIdFromFrame(frame: unknown[]): string {
  const candidate = frame[1];
  if (
    typeof candidate === "object" &&
    candidate !== null &&
    "id" in candidate &&
    typeof candidate.id === "string"
  ) {
    return candidate.id;
  }
  return "";
}

function stringOrEmpty(value: unknown): string {
  return typeof value === "string" ? value : "";
}
