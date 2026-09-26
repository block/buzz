import { createProcedureId, emptyRunbook } from "./serialize";
import type {
  SiteRunbook,
  SiteRunbookProcedure,
} from "./types";

export function setAgentBrief(
  runbook: SiteRunbook | null,
  agentBrief: string,
  now = Date.now(),
): SiteRunbook {
  const base = runbook ?? emptyRunbook(now);
  return {
    ...base,
    agentBrief,
    updatedAt: now,
  };
}

export function proposeProcedure(
  runbook: SiteRunbook | null,
  input: {
    title: string;
    steps: string;
    sourceAgent?: string;
    sourceChannel?: string;
    id?: string;
  },
  now = Date.now(),
): { runbook: SiteRunbook; procedure: SiteRunbookProcedure } {
  const base = runbook ?? emptyRunbook(now);
  const title = input.title.trim();
  if (!title) {
    throw new Error("Procedure title is required");
  }
  const procedure: SiteRunbookProcedure = {
    id: input.id?.trim() || createProcedureId(),
    title,
    steps: input.steps,
    status: "pending",
    createdAt: now,
    updatedAt: now,
  };
  if (input.sourceAgent?.trim()) {
    procedure.sourceAgent = input.sourceAgent.trim();
  }
  if (input.sourceChannel?.trim()) {
    procedure.sourceChannel = input.sourceChannel.trim();
  }
  return {
    runbook: {
      ...base,
      procedures: [...base.procedures, procedure],
      updatedAt: now,
    },
    procedure,
  };
}

export function acceptProcedure(
  runbook: SiteRunbook,
  procedureId: string,
  now = Date.now(),
): SiteRunbook {
  return {
    ...runbook,
    updatedAt: now,
    procedures: runbook.procedures.map((procedure) =>
      procedure.id === procedureId
        ? {
            ...procedure,
            status: "active" as const,
            updatedAt: now,
            acceptedAt: now,
          }
        : procedure,
    ),
  };
}

export function rejectProcedure(
  runbook: SiteRunbook,
  procedureId: string,
): SiteRunbook {
  return {
    ...runbook,
    updatedAt: Date.now(),
    procedures: runbook.procedures.filter(
      (procedure) => procedure.id !== procedureId,
    ),
  };
}

export function archiveProcedure(
  runbook: SiteRunbook,
  procedureId: string,
  now = Date.now(),
): SiteRunbook {
  return {
    ...runbook,
    updatedAt: now,
    procedures: runbook.procedures.map((procedure) =>
      procedure.id === procedureId
        ? { ...procedure, status: "archived" as const, updatedAt: now }
        : procedure,
    ),
  };
}

export function deleteProcedure(
  runbook: SiteRunbook,
  procedureId: string,
  now = Date.now(),
): SiteRunbook {
  return {
    ...runbook,
    updatedAt: now,
    procedures: runbook.procedures.filter(
      (procedure) => procedure.id !== procedureId,
    ),
  };
}

export function updateProcedure(
  runbook: SiteRunbook,
  procedureId: string,
  patch: { title?: string; steps?: string },
  now = Date.now(),
): SiteRunbook {
  return {
    ...runbook,
    updatedAt: now,
    procedures: runbook.procedures.map((procedure) => {
      if (procedure.id !== procedureId) return procedure;
      const title =
        patch.title !== undefined ? patch.title.trim() : procedure.title;
      return {
        ...procedure,
        title: title || procedure.title,
        steps: patch.steps !== undefined ? patch.steps : procedure.steps,
        updatedAt: now,
      };
    }),
  };
}

/** Merge community payload under a pin key without wiping local pending. */
export function mergeCommunityRunbook(
  existing: SiteRunbook | null,
  incoming: SiteRunbook,
  now = Date.now(),
): SiteRunbook {
  const localPending =
    existing?.procedures.filter((procedure) => procedure.status === "pending") ??
    [];
  const localArchived =
    existing?.procedures.filter(
      (procedure) => procedure.status === "archived",
    ) ?? [];
  const incomingIds = new Set(incoming.procedures.map((p) => p.id));
  const archivedKeep = localArchived.filter((p) => !incomingIds.has(p.id));
  const pendingKeep = localPending.filter((p) => !incomingIds.has(p.id));
  return {
    agentBrief: incoming.agentBrief,
    procedures: [
      ...incoming.procedures.map((p) => ({ ...p, status: "active" as const })),
      ...pendingKeep,
      ...archivedKeep,
    ],
    updatedAt: now,
  };
}
