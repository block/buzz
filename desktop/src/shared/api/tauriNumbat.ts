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

export type GuardianNumbatStatus = {
  state: "not_active" | "active" | "tampered" | "error";
  provenance: "none" | "external_unmanaged" | "buzz_managed";
  version: string | null;
  digestSuffix: string | null;
  rollbackAvailable: boolean;
  target: string;
  detail: string;
};

export function getGuardianNumbatStatus(): Promise<GuardianNumbatStatus> {
  return invokeTauri<GuardianNumbatStatus>("get_guardian_numbat_status");
}

export function activateGuardianNumbat(): Promise<GuardianNumbatStatus> {
  return invokeTauri<GuardianNumbatStatus>("activate_guardian_numbat");
}

export function installGuardianNumbat(): Promise<GuardianNumbatStatus> {
  return invokeTauri<GuardianNumbatStatus>("install_guardian_numbat");
}

export function cancelGuardianNumbatInstall(): Promise<boolean> {
  return invokeTauri<boolean>("cancel_guardian_numbat_install");
}

export function deactivateGuardianNumbat(): Promise<GuardianNumbatStatus> {
  return invokeTauri<GuardianNumbatStatus>("deactivate_guardian_numbat");
}

export function rollbackGuardianNumbat(): Promise<GuardianNumbatStatus> {
  return invokeTauri<GuardianNumbatStatus>("rollback_guardian_numbat");
}

export function uninstallGuardianNumbat(): Promise<GuardianNumbatStatus> {
  return invokeTauri<GuardianNumbatStatus>("uninstall_guardian_numbat");
}

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
