import { invokeTauri } from "@/shared/api/tauri";

export type ShepherdEvidenceEvent = {
  sequence: number;
  effectType: string;
  scope?: string | null;
  phase?: string | null;
  binding?: string | null;
  path?: string | null;
  operationId?: string | null;
  payloadSha256: string;
};

export type ShepherdEvidenceEnvelope = {
  schema: string;
  source: string;
  coverage: string;
  sourceRunRef?: string | null;
  totalEffects: number;
  effectTypes: string[];
  events: ShepherdEvidenceEvent[];
};

export type StoredShepherdEvidence = {
  agentPubkey: string;
  channelId: string;
  sessionId: string;
  turnId?: string | null;
  sourceRunRef: string;
  importedAt: number;
  evidence: ShepherdEvidenceEnvelope;
};

export async function importShepherdEvidence(input: {
  agentPubkey: string;
  channelId: string;
  sessionId: string;
  turnId?: string | null;
  sourceRunRef: string;
  exportJson: string;
}): Promise<StoredShepherdEvidence> {
  return invokeTauri("import_shepherd_evidence", { request: input });
}

export async function readShepherdEvidence(
  agentPubkey: string,
  channelId: string,
  sessionId: string,
): Promise<StoredShepherdEvidence[]> {
  return invokeTauri("read_shepherd_evidence", {
    agentPubkey,
    channelId,
    sessionId,
  });
}

export async function settleShepherdRun(
  workspacePath: string,
  sourceRunRef: string,
  action: "select" | "apply" | "discard",
): Promise<{ action: string; sourceRunRef: string; message: string }> {
  return invokeTauri("settle_shepherd_run", {
    workspacePath,
    sourceRunRef,
    action,
  });
}
