// Temporary renderer profiling harness (never merges) — record schema.
//
// Every record carries a shared envelope plus a `type` discriminant. One JSONL
// line is written per record; the analyzer keys off `type`.

/** Fields present on every profiling record. */
export type ProfileEnvelope = {
  /** `performance.now()` at record time — monotonic, sub-ms, for ordering/deltas. */
  t: number;
  /** `Date.now()` wall clock — for correlating with Will's felt lag. */
  wall: number;
  /** Per-launch session id so multiple runs never interleave. */
  sid: string;
  /** Milliseconds since the harness started this session. */
  up: number;
};

export type StallRecord = ProfileEnvelope & {
  type: "stall";
  /** Observed-minus-expected timer fire, ms (the main-thread block). */
  dur: number;
};

export type InputRecord = ProfileEnvelope & {
  type: "input";
  kind: "keydown" | "pointerdown";
  /** Capture-listener receipt → next painted frame, ms (single-clock). */
  latency: number;
  /**
   * Pre-dispatch delay: `event.timeStamp` → capture-listener receipt, ms.
   * `null` when that delta is implausible (a cross-clock basis mismatch).
   * Total felt latency is `latency + (queued ?? 0)`.
   */
  queued: number | null;
};

export type IpcRecord = ProfileEnvelope & {
  type: "ipc";
  cmd: string;
  /** Invoke call → resolve/reject, ms. */
  dur: number;
  ok: boolean;
};

export type IpcPendingRecord = ProfileEnvelope & {
  type: "ipc_pending";
  cmd: string;
  /** Age of a still-outstanding invoke, ms — the deadlock detector. */
  age: number;
};

export type RelayReqRecord = ProfileEnvelope & {
  type: "relay_req";
  /** RelayClient fetch method name — the REQ→EOSE round-trip label. */
  op: string;
  /** Method call → resolve (EOSE), ms. */
  dur: number;
  ok: boolean;
};

export type RelayPubRecord = ProfileEnvelope & {
  type: "relay_pub";
  /** RelayClient publish method name — the send→ack round-trip label. */
  op: string;
  /** Method call → resolve (relay OK), ms. */
  dur: number;
  ok: boolean;
};

export type CensusRecord = ProfileEnvelope & {
  type: "census";
  /** Total observer events retained across all agents. */
  observerEvents: number;
  observerAgents: number;
  /** Largest single-agent retained event count. */
  observerMaxPerAgent: number;
  /** Total archived observer events across all channels. */
  archiveEvents: number;
  transcripts: number;
  /** React Query cache entry count. */
  queryCache: number;
  /** Live DOM node count. */
  domNodes: number;
};

export type ProfileRecord =
  | StallRecord
  | InputRecord
  | IpcRecord
  | IpcPendingRecord
  | RelayReqRecord
  | RelayPubRecord
  | CensusRecord;

/** Payload the recorder builds internally, before the envelope is stamped. */
export type ProfileEvent = ProfileRecord extends infer R
  ? R extends ProfileRecord
    ? Omit<R, keyof ProfileEnvelope>
    : never
  : never;
