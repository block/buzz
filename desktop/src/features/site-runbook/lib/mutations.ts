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

function titlesMatch(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

/** Human Persist/lock toggle. */
export function setProcedurePersisted(
  runbook: SiteRunbook,
  procedureId: string,
  persisted: boolean,
  now = Date.now(),
): SiteRunbook {
  return {
    ...runbook,
    updatedAt: now,
    procedures: runbook.procedures.map((procedure) =>
      procedure.id === procedureId
        ? { ...procedure, persisted, updatedAt: now }
        : procedure,
    ),
  };
}

/**
 * Agent learn→write: auto-activate (no human Accept gate).
 * Replaces a non-persisted procedure with the same title; rejects if persisted.
 */
export function upsertAgentProcedure(
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
  const steps = input.steps;
  if (!String(steps ?? "").trim()) {
    throw new Error("Procedure steps are required");
  }

  const byId = input.id?.trim()
    ? base.procedures.find((p) => p.id === input.id!.trim())
    : undefined;
  const byTitle = base.procedures.find((p) => titlesMatch(p.title, title));
  const existing = byId ?? byTitle;

  if (existing?.persisted) {
    throw new Error(
      `Procedure "${existing.title}" is persisted (human lock); agents cannot modify it`,
    );
  }

  const procedure: SiteRunbookProcedure = {
    id: existing?.id || input.id?.trim() || createProcedureId(),
    title,
    steps,
    status: "active",
    createdAt: existing?.createdAt ?? now,
    updatedAt: now,
    acceptedAt: existing?.acceptedAt ?? now,
    persisted: false,
  };
  if (input.sourceAgent?.trim()) {
    procedure.sourceAgent = input.sourceAgent.trim();
  } else if (existing?.sourceAgent) {
    procedure.sourceAgent = existing.sourceAgent;
  }
  if (input.sourceChannel?.trim()) {
    procedure.sourceChannel = input.sourceChannel.trim();
  } else if (existing?.sourceChannel) {
    procedure.sourceChannel = existing.sourceChannel;
  }

  const procedures = existing
    ? base.procedures.map((p) => (p.id === existing.id ? procedure : p))
    : [...base.procedures, procedure];

  return {
    runbook: {
      ...base,
      procedures,
      updatedAt: now,
    },
    procedure,
  };
}

/**
 * @deprecated Prefer upsertAgentProcedure (auto-activates). Kept for legacy
 * pending UI / human draft flows.
 */
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
  return upsertAgentProcedure(runbook, input, now);
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
  const target = runbook.procedures.find((p) => p.id === procedureId);
  if (target?.persisted) {
    throw new Error("Persisted procedures cannot be rejected/removed this way");
  }
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
  // Humans may delete persisted items from the UI; agents never call this.
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
  // Human UI path — may edit persisted items (lock is agent-facing only).
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

/** Merge community payload under a pin key without wiping local pending/persisted. */
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
  const localPersisted =
    existing?.procedures.filter((procedure) => procedure.persisted) ?? [];
  const incomingIds = new Set(incoming.procedures.map((p) => p.id));
  const archivedKeep = localArchived.filter((p) => !incomingIds.has(p.id));
  const pendingKeep = localPending.filter((p) => !incomingIds.has(p.id));
  // Prefer local persisted copies over incoming when ids collide.
  const persistedById = new Map(localPersisted.map((p) => [p.id, p]));
  const mergedIncoming = incoming.procedures.map((p) => {
    const locked = persistedById.get(p.id);
    if (locked) return { ...locked, status: "active" as const };
    return {
      ...p,
      status: "active" as const,
      persisted: p.persisted ?? false,
    };
  });
  const persistedOnlyLocal = localPersisted.filter(
    (p) => !incomingIds.has(p.id),
  );
  return {
    agentBrief: incoming.agentBrief,
    procedures: [
      ...mergedIncoming,
      ...persistedOnlyLocal,
      ...pendingKeep,
      ...archivedKeep,
    ],
    updatedAt: now,
  };
}
