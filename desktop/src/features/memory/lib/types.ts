export type NodeEntityType =
  | "project"
  | "chat"
  | "agent"
  | "file"
  | "concept"
  | "decision"
  | "symbol"
  | "technology"
  | (string & {});

export type EdgeRelationType =
  | "authored_by"
  | "worked_on"
  | "mentions"
  | "depends_on"
  | "decided_in"
  | "supersedes"
  | (string & {});

export interface GraphNodeDto {
  id: string;
  name: string;
  entity_type: NodeEntityType;
  description: string;
  metadata?: Record<string, unknown>;
  created_at: string;
  updated_at?: string;
}

export interface GraphLinkDto {
  id: string;
  source: string;
  target: string;
  relation_type: EdgeRelationType;
  confidence: number;
  valid_at: string;
  invalid_at?: string | null;
  is_active: boolean;
  provenance_chunk_id?: string | null;
}

export interface SimNode extends GraphNodeDto {
  x: number;
  y: number;
  vx: number;
  vy: number;
  fx?: number | null;
  fy?: number | null;
  radius: number;
  color: string;
  glowColor: string;
  pulsePhase?: number;
}

export interface SimLink extends GraphLinkDto {
  sourceNode?: SimNode;
  targetNode?: SimNode;
}

export interface GraphPhysicsConfig {
  chargeRepulsion: number;
  linkDistance: number;
  centerGravity: number;
  collisionRadius: number;
  velocityDecay: number;
}

export interface BrainStats {
  monitored_workspaces: number;
  total_files_indexed: number;
  total_chunks: number;
  vector_index_status: "healthy" | "indexing" | "syncing" | "degraded";
  connected_agents_count: number;
  cloud_sync_state: "local" | "synced" | "syncing" | "offline";
  last_synced_at?: string | null;
}

export interface SuperRagMatch {
  id: string;
  node_id: string;
  name: string;
  entity_type: NodeEntityType;
  snippet: string;
  confidence: number;
  path?: string;
}
