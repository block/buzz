import { invokeTauri } from "./tauri";

export type NumbatFindingSeverity = "low" | "medium" | "high" | "critical";

export type NumbatFinding = {
  findingId: string;
  ruleId: string;
  title: string;
  severity: NumbatFindingSeverity;
  detectedAt: string;
  sourceAgent: string;
  sessionId: string | null;
  channelId: string | null;
  turnId: string | null;
  evidenceCount: number;
};

export type NumbatFindingBatch = {
  nextOffset: number;
  reset: boolean;
  rejectedRecords: number;
  health: {
    state: "active" | "configured" | "disconnected" | "unsupported" | "stale";
    detail: string;
  };
  findings: NumbatFinding[];
};

export function readNumbatFindings(
  agentPubkey: string,
  offset: number,
  sessionId: string | null,
  channelId: string | null,
  turnId: string | null,
): Promise<NumbatFindingBatch> {
  return invokeTauri<NumbatFindingBatch>("read_numbat_findings", {
    agentPubkey,
    offset,
    sessionId,
    channelId,
    turnId,
  });
}
