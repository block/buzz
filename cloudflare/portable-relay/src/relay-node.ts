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
  authorizeDirect,
  eventVisibleToReader,
  KIND_AUTH,
  KIND_HTTP_AUTH,
  PROOF_RETENTION_SECS,
  verifyNip42At,
  verifyNip98At,
  type DenialCode,
} from "./identity";
import {
  evaluateDeclarations,
  KIND_SYNC_DECLARATION,
  type DeclarationConfig,
} from "./declarations";
import {
  buildPulseEvent,
  KIND_BEACON_PULSE,
  KIND_BEACON_RESPONSE,
  parseBeaconResponse,
  witnessSecretFromEnv,
  witnessPubkeyFromEnv,
  DEFAULT_RECOGNITION_WINDOW_SECS,
  type BeaconStance,
  type PulseRecognition,
  type PulseSessions,
} from "./pulse";
import {
  eventFromUnknown,
  filtersFromUnknown,
  ProtocolInputError,
} from "./protocol";
import {
  MAX_READ_BATCH,
  parsePeerTrust,
  parseStreamExports,
  READ_CURSOR_PREFIX,
  type ReplicationBatchWire,
  type ReplicationOutcomeWire,
  type ReplicationPeerTrust,
  type ReplicationReadRequest,
  type ReplicationReceiptWire,
  type ReplicationRecordWire,
  type StreamExport,
} from "./replication";

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

interface ConnectionRow extends Record<string, SqlStorageValue> {
  connection_id: string;
  challenge: string;
  audience: string;
  principal_pubkey: string | null;
}

interface BeaconResponseRow extends Record<string, SqlStorageValue> {
  responder_pubkey: string;
  stance: BeaconStance;
}

interface BeaconRoundRow extends Record<string, SqlStorageValue> {
  pulse_id: string;
  head: string;
  created_at: number;
}

/** NIP-98 evidence captured by the Worker for one HTTP request. */
export interface HttpAuthEvidence {
  authorization: string | null;
  url: string;
  method: string;
  payloadSha256Hex: string;
}

/** RPC outcome: either an identity denial or the operation result. */
export type RpcOutcome<T> = { denied: DenialCode } | { result: T };

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
      CREATE TABLE IF NOT EXISTS artifact_refs (
        sha TEXT NOT NULL,
        event_id TEXT NOT NULL,
        PRIMARY KEY (sha, event_id)
      ) WITHOUT ROWID, STRICT
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS maintenance (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
      ) WITHOUT ROWID, STRICT
    `);
    this.#backfillArtifactRefs();
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS subscriptions (
        connection_id TEXT NOT NULL,
        subscription_id TEXT NOT NULL,
        filters_json TEXT NOT NULL,
        PRIMARY KEY (connection_id, subscription_id)
      ) WITHOUT ROWID, STRICT
    `);
    // Security state is object-local and never event history. Consumed
    // proofs are durable so a replayed proof fails closed even after
    // eviction reconstructs this object within the freshness window.
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS consumed_proofs (
        proof_id TEXT PRIMARY KEY,
        consumed_at INTEGER NOT NULL
      ) WITHOUT ROWID, STRICT
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS connections (
        connection_id TEXT PRIMARY KEY,
        challenge TEXT NOT NULL,
        audience TEXT NOT NULL,
        principal_pubkey TEXT
      ) WITHOUT ROWID, STRICT
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS beacon_rounds (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        pulse_id TEXT NOT NULL UNIQUE,
        head TEXT NOT NULL,
        created_at INTEGER NOT NULL
      ) STRICT
    `);
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS beacon_responses (
        pulse_id TEXT NOT NULL,
        responder_pubkey TEXT NOT NULL,
        stance TEXT NOT NULL CHECK (
          stance IN ('recognize', 'advanced', 'conflict', 'diverged', 'unsatisfied')
        ),
        created_at INTEGER NOT NULL,
        PRIMARY KEY (pulse_id, responder_pubkey)
      ) WITHOUT ROWID, STRICT
    `);
    // Ingest provenance is adapter metadata for provenance-selected exports;
    // added later than the base schema, so the column is created lazily.
    try {
      this.#sql.exec("ALTER TABLE event_journal ADD COLUMN ingest_source TEXT");
    } catch {
      // Column already exists.
    }
    // Destination-side replication checkpoints are operational state, kept
    // outside event history; the source remains the cursor's owner.
    this.#sql.exec(`
      CREATE TABLE IF NOT EXISTS source_checkpoints (
        source TEXT PRIMARY KEY,
        cursor TEXT NOT NULL
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
  submitEvent(
    stableNodeKey: string,
    event: Event,
    auth?: HttpAuthEvidence,
  ): RpcOutcome<WriteResult> {
    this.initializeNode(stableNodeKey);
    const outcome = this.#authenticateHttp(auth);
    if ("denied" in outcome) {
      return outcome;
    }
    if (outcome.principal !== null) {
      const denial = authorizeDirect(outcome.principal, event);
      if (denial !== null) {
        return { denied: denial };
      }
    }
    return { result: this.#applyEvent(event, null, outcome.principal) };
  }

  /**
   * Returns the effective event set selected by NIP-01 filters.
   */
  queryEvents(
    stableNodeKey: string,
    filters: Filter[],
    auth?: HttpAuthEvidence,
  ): RpcOutcome<Event[]> {
    this.initializeNode(stableNodeKey);
    const outcome = this.#authenticateHttp(auth);
    if ("denied" in outcome) {
      return outcome;
    }
    return { result: this.#queryEffective(filters, outcome.principal) };
  }

  /**
   * Counts effective events matching any supplied NIP-01 filter.
   */
  countEvents(
    stableNodeKey: string,
    filters: Filter[],
    auth?: HttpAuthEvidence,
  ): RpcOutcome<number> {
    this.initializeNode(stableNodeKey);
    const outcome = this.#authenticateHttp(auth);
    if ("denied" in outcome) {
      return outcome;
    }
    return { result: this.#countEffective(filters, outcome.principal) };
  }

  /**
   * Ingests a batch of replicated records from an authenticated peer.
   *
   * Replication always requires peer evidence, independent of the client
   * identity mode: the NIP-98 signing key must be a destination-configured
   * verification key for the batch's source stream. Records apply in order
   * through the normal ingest pipeline; receipts mirror the laptop sink's
   * outcome mapping so orchestrators can advance checkpoints identically.
   */
  ingestReplication(
    stableNodeKey: string,
    records: ReplicationRecordWire[],
    auth: HttpAuthEvidence,
  ): RpcOutcome<ReplicationReceiptWire[]> {
    this.initializeNode(stableNodeKey);
    const verified = this.#verifyHttpEvidence(auth);
    if ("denied" in verified) {
      return verified;
    }
    if (records.length === 0) {
      return { result: [] };
    }
    const source = records[0].source;
    const trust = this.#resolvedPeers()[source];
    if (trust === undefined) {
      return { denied: "peer_unbound" };
    }
    if (!trust.verification_keys.includes(verified.principal)) {
      return { denied: "source_mismatch" };
    }

    const receipts: ReplicationReceiptWire[] = [];
    let checkpoint: string | null = null;
    for (const record of records) {
      const receipt = this.#applyReplicationRecord(source, record);
      receipts.push(receipt);
      if (receipt.outcome.status === "rejected") {
        break;
      }
      checkpoint = record.cursor;
    }
    if (checkpoint !== null) {
      this.#sql.exec(
        `INSERT INTO source_checkpoints (source, cursor) VALUES (?, ?)
         ON CONFLICT (source) DO UPDATE SET cursor = excluded.cursor`,
        source,
        checkpoint,
      );
    }
    return { result: receipts };
  }

  /**
   * Serves a page of this node's journal as a replication export — the
   * rendezvous role: peers the operator has authorized as readers may drain
   * streams this node custodies, without the original source being online.
   *
   * Reader trust and stream exports are operator configuration — owner-signed
   * declaration heads in this node's own journal when present, env bootstrap
   * (BUZZ_REPLICATION_READERS / BUZZ_REPLICATION_STREAMS) otherwise; cursors
   * belong to this node's own cursor space, independent of the cursors
   * records carried when they were ingested.
   */
  readReplication(
    stableNodeKey: string,
    request: ReplicationReadRequest,
    auth: HttpAuthEvidence,
  ): RpcOutcome<ReplicationBatchWire> {
    this.initializeNode(stableNodeKey);
    const verified = this.#verifyHttpEvidence(auth);
    if ("denied" in verified) {
      return verified;
    }
    const exported = this.#resolvedStreams()[request.source];
    const readers = this.#resolvedReaders()[request.source];
    if (exported === undefined || readers === undefined) {
      return { denied: "peer_unbound" };
    }
    if (!readers.verification_keys.includes(verified.principal)) {
      return { denied: "source_mismatch" };
    }

    const after =
      request.cursor === null
        ? 0
        : Number.parseInt(request.cursor.slice(READ_CURSOR_PREFIX.length), 10);
    const scanLimit = Math.min(request.limit, MAX_READ_BATCH);
    const rows = Array.from(
      this.#sql.exec<
        {
          sequence: number;
          event_json: string;
          ingest_source: string | null;
        } & Record<string, SqlStorageValue>
      >(
        `SELECT sequence, event_json, ingest_source FROM event_journal
         WHERE sequence > ? ORDER BY sequence ASC LIMIT ?`,
        after,
        scanLimit,
      ),
    );
    const lastScanned =
      rows.length > 0 ? rows[rows.length - 1].sequence : after;
    const records = rows
      .map((row) => ({
        sequence: row.sequence,
        ingestSource: row.ingest_source,
        event: JSON.parse(row.event_json) as Event,
      }))
      .filter((row) => {
        if (exported.mode === "mirror") {
          return true;
        }
        if (exported.mode === "filter") {
          return matchFilters(exported.filters as Filter[], row.event);
        }
        return row.ingestSource === exported.source;
      })
      .map((row) => ({
        source: request.source,
        cursor: `${READ_CURSOR_PREFIX}${row.sequence}`,
        event: row.event,
      }));
    return {
      result: {
        records,
        next_cursor: `${READ_CURSOR_PREFIX}${lastScanned}`,
        caught_up: rows.length < scanLimit,
      },
    };
  }

  /**
   * Owner-signed declaration heads evaluated into configuration. Without
   * both anchors (BUZZ_OWNER_PUBKEY and BUZZ_NODE_LABEL) every domain is
   * unclaimed and env bootstrap config governs, matching pre-declaration
   * behavior. The node label keeps a replicated journal safe to evaluate
   * everywhere: this node applies only heads n-tagged for it.
   */
  #ownerDeclarations(): DeclarationConfig {
    const owner: string = this.env.BUZZ_OWNER_PUBKEY ?? "";
    const nodeLabel: string = this.env.BUZZ_NODE_LABEL ?? "";
    if (owner === "" || nodeLabel === "") {
      return { peers: null, readers: null, streams: null };
    }
    return evaluateDeclarations(
      this.#declarationHeadEvents(),
      owner,
      nodeLabel,
    );
  }

  /** Effective journal events that may carry sync declarations. */
  #declarationHeadEvents(): Event[] {
    // The LIKE clause is a coarse prefilter only (the kind number appears
    // somewhere in the JSON); consumers re-check kind and author.
    const rows = Array.from(
      this.#sql.exec<{ event_json: string } & Record<string, SqlStorageValue>>(
        `SELECT event_json FROM event_journal
         WHERE effective = 1 AND event_json LIKE ?`,
        `%${KIND_SYNC_DECLARATION}%`,
      ),
    );
    return rows.map((row) => JSON.parse(row.event_json) as Event);
  }

  #resolvedPeers(): Record<string, ReplicationPeerTrust> {
    return (
      this.#ownerDeclarations().peers ??
      parsePeerTrust(this.env.BUZZ_REPLICATION_PEERS)
    );
  }

  #resolvedReaders(): Record<string, ReplicationPeerTrust> {
    return (
      this.#ownerDeclarations().readers ??
      parsePeerTrust(this.env.BUZZ_REPLICATION_READERS)
    );
  }

  #resolvedStreams(): Record<string, StreamExport> {
    return (
      this.#ownerDeclarations().streams ??
      parseStreamExports(this.env.BUZZ_REPLICATION_STREAMS)
    );
  }

  /**
   * The Beacon pulse: this node's signed witness statement of the state it
   * currently holds — journal head, replication checkpoints, and the
   * declaration heads it applies. Synthesized fresh on every call (a pulse
   * is an answer, not stored state) and null when no witness key is
   * configured. Returns the current head alongside the event so emission
   * can advance the witnessed chain without re-reading the journal.
   */
  #currentPulse(): { event: Event; head: string | null } | null {
    const secret = witnessSecretFromEnv(this.env.BUZZ_NODE_SECRET);
    if (secret === null) {
      return null;
    }
    const headRow = Array.from(
      this.#sql.exec<
        { sequence: number; event_id: string } & Record<string, SqlStorageValue>
      >(
        `SELECT sequence, event_id FROM event_journal
         ORDER BY sequence DESC LIMIT 1`,
      ),
    )[0];
    const head = headRow?.event_id ?? null;
    const witnessedHead = this.#maintenanceValue("witnessed_head");
    const witnessedPrev = this.#maintenanceValue("witnessed_prev");
    const checkpoints: Record<string, string> = {};
    for (const row of this.#sql.exec<
      { source: string; cursor: string } & Record<string, SqlStorageValue>
    >("SELECT source, cursor FROM source_checkpoints ORDER BY source")) {
      checkpoints[row.source] = row.cursor;
    }
    const declarations = this.#ownerDeclarations();
    const agreements: Record<string, string> = {};
    const owner: string = this.env.BUZZ_OWNER_PUBKEY ?? "";
    const nodeLabel: string = this.env.BUZZ_NODE_LABEL ?? "";
    if (owner !== "" && nodeLabel !== "") {
      for (const event of this.#declarationHeadEvents()) {
        if (event.kind !== KIND_SYNC_DECLARATION || event.pubkey !== owner) {
          continue;
        }
        const dTag = event.tags.find((tag) => tag[0] === "d")?.[1];
        const nTag = event.tags.find((tag) => tag[0] === "n")?.[1];
        if (dTag === undefined || nTag !== nodeLabel) {
          continue;
        }
        agreements[dTag] = event.id;
      }
    }
    const sessions = this.#pulseSessions();
    const recognition = this.#pulseRecognition(head);
    const event = buildPulseEvent(
      {
        stableNodeKey: this.describeNode()?.stableNodeKey ?? "",
        nodeLabel,
        journal: { sequence: headRow?.sequence ?? 0, head },
        // The witnessed chain: `previous` is the head recognized before the
        // current one. Idle reads (head already witnessed) report the link
        // one step back so the chain stays a chain, not a self-loop.
        previous: head === witnessedHead ? witnessedPrev : witnessedHead,
        checkpoints,
        agreements,
        governance: {
          peers: declarations.peers === null ? "bootstrap" : "journal",
          readers: declarations.readers === null ? "bootstrap" : "journal",
          streams: declarations.streams === null ? "bootstrap" : "journal",
        },
        ...(sessions === undefined ? {} : { sessions }),
        ...(recognition === undefined ? {} : { recognition }),
      },
      secret,
      nowSecs(),
    );
    return { event, head };
  }

  /**
   * Who may observe the pulse. It reveals journal metadata (head IDs,
   * cursors, agreement heads), so under required identity it is addressed
   * to the parties of this node's agreements: the owner and any declared
   * peer or reader verification key. On an open node it is open.
   */
  #pulseVisibleTo(reader: string | null): boolean {
    if (!this.#authRequired()) {
      return true;
    }
    if (reader === null) {
      return false;
    }
    const owner: string = this.env.BUZZ_OWNER_PUBKEY ?? "";
    if (owner !== "" && reader === owner) {
      return true;
    }
    const hasKey = (trust: ReplicationPeerTrust): boolean =>
      trust.verification_keys.includes(reader);
    return (
      Object.values(this.#resolvedPeers()).some(hasKey) ||
      Object.values(this.#resolvedReaders()).some(hasKey)
    );
  }

  /**
   * Publishes a fresh pulse to live subscribers after a journal transition
   * and advances the witnessed chain. Emission is the only place the chain
   * moves: synthesis on read observes, never transitions.
   */
  #emitPulse(): void {
    const pulse = this.#currentPulse();
    if (pulse === null) {
      return;
    }
    this.#registerPulse(pulse.event, pulse.head);
    this.#publishLive(pulse.event);
    const witnessedHead = this.#maintenanceValue("witnessed_head");
    if (pulse.head === null || pulse.head === witnessedHead) {
      return;
    }
    this.ctx.storage.transactionSync(() => {
      if (witnessedHead !== null) {
        this.#setMaintenanceValue("witnessed_prev", witnessedHead);
      }
      this.#setMaintenanceValue("witnessed_head", pulse.head as string);
    });
  }

  #maintenanceValue(key: string): string | null {
    const row = Array.from(
      this.#sql.exec<{ value: string } & Record<string, SqlStorageValue>>(
        "SELECT value FROM maintenance WHERE key = ?",
        key,
      ),
    )[0];
    return row === undefined ? null : row.value;
  }

  #setMaintenanceValue(key: string, value: string): void {
    this.#sql.exec(
      `INSERT INTO maintenance (key, value) VALUES (?, ?)
       ON CONFLICT (key) DO UPDATE SET value = excluded.value`,
      key,
      value,
    );
  }

  #pulseSessions(): PulseSessions | undefined {
    if (!this.#authRequired()) {
      return undefined;
    }
    const principals = new Set<string>();
    let count = 0;
    for (const ws of this.ctx.getWebSockets()) {
      const principal = this.#connectionRow(ws)?.principal_pubkey ?? null;
      if (principal === null) {
        continue;
      }
      count += 1;
      principals.add(principal);
    }
    return {
      count,
      principals: Array.from(principals).sort(),
    };
  }

  #pulseRecognition(head: string | null): PulseRecognition | undefined {
    if (head === null) {
      return undefined;
    }
    const round = Array.from(
      this.#sql.exec<BeaconRoundRow>(
        `SELECT pulse_id, head, created_at
         FROM beacon_rounds
         WHERE head = ?
           AND created_at >= ?
           AND EXISTS (
             SELECT 1 FROM beacon_responses
             WHERE beacon_responses.pulse_id = beacon_rounds.pulse_id
           )
         ORDER BY sequence DESC
         LIMIT 1`,
        head,
        nowSecs() - DEFAULT_RECOGNITION_WINDOW_SECS,
      ),
    )[0];
    if (round === undefined) {
      return undefined;
    }
    const responses: Record<string, BeaconStance> = {};
    for (const row of this.#sql.exec<BeaconResponseRow>(
      `SELECT responder_pubkey, stance
       FROM beacon_responses
       WHERE pulse_id = ?
       ORDER BY responder_pubkey`,
      round.pulse_id,
    )) {
      responses[row.responder_pubkey] = row.stance;
    }
    return {
      head,
      pulse: round.pulse_id,
      responses,
      window_secs: DEFAULT_RECOGNITION_WINDOW_SECS,
    };
  }

  #registerPulse(event: Event, head: string | null): void {
    this.ctx.storage.transactionSync(() => {
      const cutoff = nowSecs() - DEFAULT_RECOGNITION_WINDOW_SECS;
      this.#sql.exec(
        `DELETE FROM beacon_responses
         WHERE pulse_id IN (
           SELECT pulse_id FROM beacon_rounds WHERE created_at < ?
         )`,
        cutoff,
      );
      this.#sql.exec("DELETE FROM beacon_rounds WHERE created_at < ?", cutoff);
      if (head !== null) {
        this.#sql.exec(
          `INSERT OR IGNORE INTO beacon_rounds (pulse_id, head, created_at)
           VALUES (?, ?, ?)`,
          event.id,
          head,
          event.created_at,
        );
      }
    });
  }

  #recordBeaconResponse(event: Event): boolean {
    const witnessPubkey = witnessPubkeyFromEnv(this.env.BUZZ_NODE_SECRET);
    const pulseId = event.tags.find((tag) => tag[0] === "e")?.[1];
    if (pulseId === undefined || witnessPubkey === null) {
      return false;
    }
    const round = Array.from(
      this.#sql.exec<BeaconRoundRow>(
        `SELECT pulse_id, head, created_at
         FROM beacon_rounds
         WHERE pulse_id = ?`,
        pulseId,
      ),
    )[0];
    if (round === undefined) {
      return false;
    }
    const parsed = parseBeaconResponse(
      event,
      {
        pulseId: round.pulse_id,
        pulseHead: round.head,
        pulseCreatedAt: round.created_at,
        witnessPubkey,
        windowSecs: DEFAULT_RECOGNITION_WINDOW_SECS,
      },
      nowSecs(),
    );
    if (parsed === null) {
      return false;
    }
    this.#sql.exec(
      `INSERT INTO beacon_responses (
         pulse_id, responder_pubkey, stance, created_at
       ) VALUES (?, ?, ?, ?)
       ON CONFLICT (pulse_id, responder_pubkey)
       DO UPDATE SET stance = excluded.stance, created_at = excluded.created_at`,
      pulseId,
      event.pubkey,
      parsed.stance,
      event.created_at,
    );
    return true;
  }

  /**
   * May this authenticated principal store a blob? Uploads accompany stream
   * pushes: the owner and admitted replication peers only.
   */
  authorizeArtifactUpload(
    stableNodeKey: string,
    auth: HttpAuthEvidence,
  ): RpcOutcome<{ principal: string }> {
    this.initializeNode(stableNodeKey);
    const verified = this.#verifyHttpEvidence(auth);
    if ("denied" in verified) {
      return verified;
    }
    const owner: string = this.env.BUZZ_OWNER_PUBKEY ?? "";
    if (owner !== "" && verified.principal === owner) {
      return { result: { principal: verified.principal } };
    }
    const admitted = Object.values(this.#resolvedPeers()).some((trust) =>
      trust.verification_keys.includes(verified.principal),
    );
    return admitted
      ? { result: { principal: verified.principal } }
      : { denied: "scope_denied" };
  }

  /**
   * Artifact access follows reference (evaluation rule 4): a blob is served
   * only to the owner or to a principal holding a read grant on a stream
   * whose journal events reference it via `x` tags. An unreferenced blob is
   * invisible to everyone but the owner.
   */
  authorizeArtifactFetch(
    stableNodeKey: string,
    sha: string,
    auth: HttpAuthEvidence,
  ): RpcOutcome<{ principal: string }> {
    this.initializeNode(stableNodeKey);
    const verified = this.#verifyHttpEvidence(auth);
    if ("denied" in verified) {
      return verified;
    }
    const owner: string = this.env.BUZZ_OWNER_PUBKEY ?? "";
    if (owner !== "" && verified.principal === owner) {
      return { result: { principal: verified.principal } };
    }
    const readers = this.#resolvedReaders();
    const streams = this.#resolvedStreams();
    const readable = Object.keys(readers).filter((stream) =>
      readers[stream].verification_keys.includes(verified.principal),
    );
    if (readable.length === 0) {
      return { denied: "scope_denied" };
    }
    const referencing = Array.from(
      this.#sql.exec<
        { event_json: string; ingest_source: string | null } & Record<
          string,
          SqlStorageValue
        >
      >(
        `SELECT j.event_json, j.ingest_source
         FROM artifact_refs r JOIN event_journal j ON j.event_id = r.event_id
         WHERE r.sha = ?`,
        sha,
      ),
    ).map((row) => ({
      event: JSON.parse(row.event_json) as Event,
      ingestSource: row.ingest_source,
    }));
    for (const stream of readable) {
      const selection = streams[stream];
      if (selection === undefined) {
        continue;
      }
      for (const row of referencing) {
        const visible =
          selection.mode === "mirror"
            ? true
            : selection.mode === "filter"
              ? matchFilters(selection.filters as Filter[], row.event)
              : row.ingestSource === selection.source;
        if (visible) {
          return { result: { principal: verified.principal } };
        }
      }
    }
    return { denied: "scope_denied" };
  }

  /** Records every well-formed `x`-tag content reference carried by an event. */
  #indexArtifactRefs(eventId: string, tags: string[][]): void {
    for (const tag of tags) {
      if (tag[0] !== "x" || typeof tag[1] !== "string") {
        continue;
      }
      const sha = tag[1].toLowerCase();
      if (!/^[0-9a-f]{64}$/.test(sha)) {
        continue;
      }
      this.#sql.exec(
        `INSERT INTO artifact_refs (sha, event_id) VALUES (?, ?)
         ON CONFLICT (sha, event_id) DO NOTHING`,
        sha,
        eventId,
      );
    }
  }

  /** One-time index of `x`-tag references already present in the journal. */
  #backfillArtifactRefs(): void {
    const done = Array.from(
      this.#sql.exec<{ value: string } & Record<string, SqlStorageValue>>(
        `SELECT value FROM maintenance WHERE key = 'artifact_refs_backfilled'`,
      ),
    )[0];
    if (done !== undefined) {
      return;
    }
    this.ctx.storage.transactionSync(() => {
      const rows = Array.from(
        this.#sql.exec<
          { event_id: string; event_json: string } & Record<
            string,
            SqlStorageValue
          >
        >(
          `SELECT event_id, event_json FROM event_journal
           WHERE event_json LIKE '%"x"%'`,
        ),
      );
      for (const row of rows) {
        this.#indexArtifactRefs(
          row.event_id,
          (JSON.parse(row.event_json) as Event).tags,
        );
      }
      this.#sql.exec(
        `INSERT INTO maintenance (key, value)
         VALUES ('artifact_refs_backfilled', '1')`,
      );
    });
  }

  #applyReplicationRecord(
    source: string,
    record: ReplicationRecordWire,
  ): ReplicationReceiptWire {
    const receipt = (
      outcome: ReplicationOutcomeWire,
    ): ReplicationReceiptWire => ({
      source: record.source,
      cursor: record.cursor,
      event_id: record.event.id,
      outcome,
    });
    const rejected = (reason: string) =>
      receipt({ status: "rejected", reason });

    if (record.source !== source) {
      return rejected("record source does not match the authenticated batch");
    }
    if (isEphemeralKind(record.event.kind)) {
      return rejected("ephemeral events are not part of durable replication");
    }
    if (
      record.event.kind === KIND_AUTH ||
      record.event.kind === KIND_HTTP_AUTH
    ) {
      return rejected("authentication events are never journaled");
    }
    const result = this.#applyEvent(record.event, source);
    if (!result.accepted) {
      return rejected(result.message);
    }
    switch (result.message) {
      case "stored":
        return receipt({ status: "stored" });
      case "duplicate":
        return receipt({ status: "duplicate" });
      case "superseded":
        return receipt({ status: "superseded" });
      default:
        return rejected(`unexpected accepted outcome: ${result.message}`);
    }
  }

  #authRequired(): boolean {
    const value: string = this.env.BUZZ_REQUIRE_AUTH ?? "";
    return value === "1" || value === "true" || value === "yes";
  }

  /**
   * Resolves the HTTP principal: `null` when this node does not require
   * authentication, the pubkey on success, or a denial outcome.
   */
  #authenticateHttp(
    auth?: HttpAuthEvidence,
  ): { principal: string | null } | { denied: DenialCode } {
    if (!this.#authRequired()) {
      return { principal: null };
    }
    return this.#verifyHttpEvidence(auth);
  }

  /** Verifies mandatory NIP-98 evidence and consumes its one-use proof. */
  #verifyHttpEvidence(
    auth?: HttpAuthEvidence,
  ): { principal: string } | { denied: DenialCode } {
    const encoded = auth?.authorization?.startsWith("Nostr ")
      ? auth.authorization.slice("Nostr ".length)
      : null;
    if (auth === undefined || encoded === null) {
      return { denied: "authentication_required" };
    }
    let proof: Event;
    try {
      proof = eventFromUnknown(JSON.parse(atob(encoded)));
    } catch {
      return { denied: "invalid_evidence" };
    }
    const verified = verifyNip98At(
      proof,
      {
        url: auth.url,
        method: auth.method,
        payloadSha256Hex: auth.payloadSha256Hex,
      },
      nowSecs(),
    );
    if (!verified.ok) {
      return { denied: verified.code };
    }
    const replay = this.#consumeProof(verified.proofId);
    if (replay !== null) {
      return { denied: replay };
    }
    return { principal: verified.pubkey };
  }

  #consumeProof(proofId: string): DenialCode | null {
    const now = nowSecs();
    const existing = Array.from(
      this.#sql.exec(
        "SELECT 1 FROM consumed_proofs WHERE proof_id = ?",
        proofId,
      ),
    );
    if (existing.length > 0) {
      return "replay_detected";
    }
    this.ctx.storage.transactionSync(() => {
      this.#sql.exec(
        "INSERT INTO consumed_proofs (proof_id, consumed_at) VALUES (?, ?)",
        proofId,
        now,
      );
      this.#sql.exec(
        "DELETE FROM consumed_proofs WHERE consumed_at < ?",
        now - PROOF_RETENTION_SECS,
      );
    });
    return null;
  }

  #applyEvent(
    event: Event,
    ingestSource: string | null = null,
    principal: string | null = null,
  ): WriteResult {
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

    if (event.kind === KIND_BEACON_RESPONSE) {
      if (
        (this.#authRequired() &&
          (principal === null || !this.#pulseVisibleTo(principal))) ||
        !this.#recordBeaconResponse(event)
      ) {
        return {
          event_id: event.id,
          accepted: false,
          message: "invalid",
        };
      }
      this.#publishLive(event);
      return {
        event_id: event.id,
        accepted: true,
        message: "ephemeral",
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
           event_id, event_json, replacement_key, effective, ingest_source
         ) VALUES (?, ?, ?, 1, ?)`,
        event.id,
        eventJson,
        candidateKey,
        ingestSource,
      );
      this.#indexArtifactRefs(event.id, event.tags);
    });
    this.#publishLive(event);
    // Every journal transition is witnessed: the pulse follows the event.
    this.#emitPulse();

    return {
      event_id: event.id,
      accepted: true,
      message: "stored",
    };
  }

  #queryEffective(filters: Filter[], reader: string | null = null): Event[] {
    if (filters.length === 0) {
      return [];
    }

    const ordered = sortEvents(this.#readableEvents(reader));
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
    // The pulse is synthesized, never stored: a filter that explicitly asks
    // for the pulse kind receives a fresh witness statement. Open filters
    // never surface it — witnessing happens on request, not by accident.
    const pulseFilters = filters.filter(
      (filter) => filter.kinds?.includes(KIND_BEACON_PULSE) ?? false,
    );
    if (pulseFilters.length > 0 && this.#pulseVisibleTo(reader)) {
      const pulse = this.#currentPulse();
      if (
        pulse !== null &&
        pulseFilters.some((filter) => matchFilter(filter, pulse.event))
      ) {
        this.#registerPulse(pulse.event, pulse.head);
        selected.set(pulse.event.id, pulse.event);
      }
    }
    return sortEvents(Array.from(selected.values()));
  }

  #countEffective(filters: Filter[], reader: string | null = null): number {
    if (filters.length === 0) {
      return 0;
    }
    return this.#readableEvents(reader).filter((event) =>
      matchFilters(filters, event),
    ).length;
  }

  #readableEvents(reader: string | null): Event[] {
    const events = this.#effectiveEvents();
    if (reader === null) {
      return events;
    }
    return events.filter((event) => eventVisibleToReader(reader, event));
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
    const connectionId = crypto.randomUUID();
    server.serializeAttachment({
      version: 1,
      connectionId,
    } satisfies ConnectionAttachment);
    if (this.#authRequired()) {
      const challenge = randomChallenge();
      const audience = `${new URL(request.url).origin.replace(/^http/, "ws")}/`;
      this.#sql.exec(
        `INSERT INTO connections (connection_id, challenge, audience, principal_pubkey)
         VALUES (?, ?, ?, NULL)`,
        connectionId,
        challenge,
        audience,
      );
      this.#send(server, ["AUTH", challenge]);
    }
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

    if (frame[0] === "AUTH") {
      this.#handleWebSocketAuth(ws, frame);
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

  #handleWebSocketAuth(ws: WebSocket, frame: unknown[]): void {
    let event: Event;
    try {
      event = eventFromUnknown(frame[1]);
    } catch {
      this.#send(ws, ["OK", "", false, "invalid_evidence"]);
      return;
    }
    if (!this.#authRequired()) {
      this.#send(ws, ["OK", event.id, false, "authentication_not_enabled"]);
      return;
    }
    const connection = this.#connectionRow(ws);
    if (connection === null) {
      this.#send(ws, ["OK", event.id, false, "authentication_required"]);
      return;
    }
    if (connection.principal_pubkey !== null) {
      this.#send(ws, ["OK", event.id, false, "replay_detected"]);
      return;
    }
    const verified = verifyNip42At(
      event,
      connection.challenge,
      connection.audience,
      nowSecs(),
    );
    if (!verified.ok) {
      this.#send(ws, ["OK", event.id, false, verified.code]);
      return;
    }
    const replay = this.#consumeProof(verified.proofId);
    if (replay !== null) {
      this.#send(ws, ["OK", event.id, false, replay]);
      return;
    }
    this.#sql.exec(
      "UPDATE connections SET principal_pubkey = ? WHERE connection_id = ?",
      verified.pubkey,
      connection.connection_id,
    );
    this.#send(ws, ["OK", event.id, true, "authenticated"]);
  }

  #handleWebSocketEvent(ws: WebSocket, frame: unknown[]): void {
    try {
      const event = eventFromUnknown(frame[1]);
      if (this.#authRequired()) {
        const principal = this.#connectionRow(ws)?.principal_pubkey ?? null;
        if (principal === null) {
          this.#send(ws, ["OK", event.id, false, "authentication_required"]);
          return;
        }
        const denial = authorizeDirect(principal, event);
        if (denial !== null) {
          this.#send(ws, ["OK", event.id, false, denial]);
          return;
        }
      }
      const principal = this.#authRequired()
        ? (this.#connectionRow(ws)?.principal_pubkey ?? null)
        : null;
      const result = this.#applyEvent(event, null, principal);
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
    let reader: string | null = null;
    if (this.#authRequired()) {
      reader = this.#connectionRow(ws)?.principal_pubkey ?? null;
      if (reader === null) {
        this.#send(ws, ["CLOSED", subscriptionId, "authentication_required"]);
        return;
      }
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

    for (const event of this.#queryEffective(filters, reader)) {
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
    const authRequired = this.#authRequired();
    for (const ws of this.ctx.getWebSockets()) {
      const attachment = this.#attachmentOrNull(ws);
      if (attachment === null) {
        continue;
      }
      if (authRequired) {
        const reader = this.#connectionRow(ws)?.principal_pubkey ?? null;
        const visible =
          event.kind === KIND_BEACON_PULSE ||
          event.kind === KIND_BEACON_RESPONSE
            ? this.#pulseVisibleTo(reader)
            : reader !== null && eventVisibleToReader(reader, event);
        if (!visible) {
          continue;
        }
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
    this.#sql.exec(
      "DELETE FROM connections WHERE connection_id = ?",
      attachment.connectionId,
    );
  }

  #connectionRow(ws: WebSocket): ConnectionRow | null {
    const attachment = this.#attachmentOrNull(ws);
    if (attachment === null) {
      return null;
    }
    const row = Array.from(
      this.#sql.exec<ConnectionRow>(
        `SELECT connection_id, challenge, audience, principal_pubkey
         FROM connections
         WHERE connection_id = ?`,
        attachment.connectionId,
      ),
    )[0];
    return row === undefined ? null : row;
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

function nowSecs(): number {
  return Math.floor(Date.now() / 1000);
}

function randomChallenge(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
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
