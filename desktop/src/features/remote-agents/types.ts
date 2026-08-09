/** Host-seat-location types for Remote Agents (layer 3).
 * Entity holon P0: birth_cert / body / public place_proof.v1
 */

export type HostAgentHealth = "online" | "stale" | "stopped" | "unknown";

/** place_proof.v1 health (host controller). Maps to HostAgentHealth in UI. */
export type PlaceProofHealth = "ok" | "degraded" | "stale" | "down";

export type SurfaceKind =
  | "desktop-local"
  | "cli-seat"
  | "host-unit"
  | "remote-view"
  | string;

export type RemoteAgentPreset =
  | "co-lab-gemma"
  | "co-lab-watch"
  | "push-nerve"
  | "status-only";

/** Public place_proof.v1 — room/mesh safe (no surface_root, pid, nsec). */
export type PlaceProofPublic = {
  schema: "place_proof.v1" | string;
  birth_cert_id?: string;
  legal_name?: string;
  seat_id?: string;
  body_id?: string | null;
  host_id?: string;
  host_role?: string;
  surface_kind?: SurfaceKind;
  surface_id?: string;
  health?: PlaceProofHealth | string;
  lease_epoch?: number;
  issued_at?: number;
  expires_at?: number;
  attestation?: string;
  runtime?: string | null;
  model?: string | null;
};

export type DualBodyError = {
  ok: false;
  error: "dual_body";
  message?: string;
  seat?: string;
  place_proof?: PlaceProofPublic;
};

export type HostAgentSeat = {
  seat_id: string;
  /** Immutable DNA (Nostr pubkey) when known */
  birth_cert_id?: string;
  pubkey?: string;
  pubkey_hint?: string;
  body_id?: string;
  lease_epoch?: number;
  runtimes?: string[];
  model?: string;
  channels?: string[];
  expected_online?: boolean;
  notes?: string;
  unit_name?: string;
  unit_pid?: number | null;
  unit_alive?: boolean;
  /** Host-local only — do not render full path in multi-user UI */
  surface_root?: string;
  surface_kind?: SurfaceKind;
  surface_id?: string;
  project_ids?: string[];
};

export type HostAgentStatus = {
  ok?: boolean;
  schema?: string;
  host_id?: string;
  host_role?: string;
  ts?: number;
  relay?: { http_code?: string; url?: string; ok?: boolean };
  ollama?: { ok?: boolean; models?: string[] };
  watchers?: { process_matches?: number; unit_pids?: number };
  seats?: HostAgentSeat[];
  error?: string;
  raw?: string;
};

export type RemoteHostConnection = {
  /** Display name, e.g. asus-g501vw */
  label: string;
  /** Base URL, e.g. http://127.0.0.1:8787 (SSH tunnel) or http://100.x.y.z:8787 */
  baseUrl: string;
  /** Bearer token for host-agentd — stored locally (v1); prefer OS keyring later */
  token: string;
  /** Default channel UUID for arm (e.g. agent-metabolism) */
  defaultRoom?: string;
};

export type RemoteAgentCardModel = {
  seatId: string;
  hostId: string;
  hostRole: string;
  model: string;
  runtimes: string[];
  channels: string[];
  expectedOnline: boolean;
  health: HostAgentHealth;
  healthLabel: string;
  relayOk: boolean;
  ollamaOk: boolean;
  /** @deprecated host-local only — prefer surfaceId in UI */
  surfaceRoot?: string;
  /** Public stable bind id (place_proof.v1) — never a full home path */
  surfaceId?: string;
  surfaceKind?: SurfaceKind;
  /** Immutable DNA short display (first 8 of pubkey) */
  birthCertShort?: string;
  birthCertId?: string;
  bodyId?: string;
  leaseEpoch?: number;
  projectIds?: string[];
  unitPid?: number | null;
  /** True when a live body exists — Arm should not invite dual spawn */
  bodyLive?: boolean;
};

export const REMOTE_AGENT_PRESETS: {
  id: RemoteAgentPreset;
  label: string;
  description: string;
}[] = [
  {
    id: "co-lab-gemma",
    label: "Co-lab + local LLM",
    description: "Watch + local-llm drafts (Ollama model)",
  },
  {
    id: "co-lab-watch",
    label: "Co-lab watch only",
    description: "Watch/admit without model cortex",
  },
  {
    id: "push-nerve",
    label: "Push nerve / Codex@home",
    description: "Codex-style push L0 on the host",
  },
  {
    id: "status-only",
    label: "Status only",
    description: "Register seat · no process yet",
  },
];

/** Suggested models for the Create remote agent dialog (host-side). */
export const REMOTE_AGENT_MODEL_OPTIONS: {
  id: string;
  label: string;
  hint: string;
}[] = [
  {
    id: "gemma3:4b",
    label: "gemma3:4b (Ollama)",
    hint: "Local on home · co-lab-gemma",
  },
  {
    id: "llama3.2:3b",
    label: "llama3.2:3b (Ollama)",
    hint: "Local fallback on home",
  },
  {
    id: "grok-4.5",
    label: "grok-4.5 (remote internal)",
    hint: "Intent for Grok 4.5 on host · full cortex later",
  },
];
