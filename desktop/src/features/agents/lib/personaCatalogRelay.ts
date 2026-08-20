import type {
  AgentPersona,
  CatalogSourceCoordinate,
  RespondToMode,
} from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

export type CatalogPersonaShareLevel = "not-shared" | "none";

type CatalogAgentProjection = {
  displayName: string;
  avatarUrl: string | null;
  systemPrompt: string;
  runtime: string | null;
  model: string | null;
  provider: string | null;
  namePool: string[];
  respondTo: RespondToMode | null;
  parallelism: number | null;
};

export type PersonaCatalogPublication = {
  eventId: string;
  ownerPubkey: string;
  sourcePersonaId: string;
  createdAt: number;
  agent: CatalogAgentProjection;
};

export type CatalogPersona = AgentPersona & {
  catalogSource: CatalogSourceCoordinate & {
    /** The publication event this projection was built from. */
    eventId: string;
    /** Whether the current identity published it. */
    isOwn: boolean;
  };
};

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Fetch the active community catalog through the shared native relay session.
 * Relay scoping, paging, signature verification, and head selection are native;
 * this boundary intentionally accepts no caller-supplied relay or identity.
 */
export function fetchPersonaCatalogPublications(): Promise<
  PersonaCatalogPublication[]
> {
  return invokeTauri<PersonaCatalogPublication[]>("fetch_persona_catalog");
}

function publicationToPersona(
  publication: PersonaCatalogPublication,
  localPersona: AgentPersona | undefined,
  isOwn: boolean,
): CatalogPersona {
  const timestamp = new Date(publication.createdAt * 1_000).toISOString();
  // The publication remains authoritative for catalog presentation. An added
  // local copy contributes only the linkage id and selected state; merging the
  // whole copy would leak local edits (notably its avatar) into the publisher's
  // catalog entry.
  const basePersona: AgentPersona = {
    id:
      localPersona?.id ??
      `catalog:${publication.ownerPubkey}:${publication.sourcePersonaId}`,
    displayName: publication.agent.displayName,
    avatarUrl: publication.agent.avatarUrl,
    systemPrompt: publication.agent.systemPrompt,
    runtime: publication.agent.runtime,
    model: publication.agent.model,
    provider: publication.agent.provider,
    namePool: publication.agent.namePool,
    isBuiltIn: false,
    isActive: localPersona?.isActive ?? false,
    shared: true,
    sourceTeam: null,
    envVars: {},
    respondTo: publication.agent.respondTo,
    respondToAllowlist: [],
    parallelism: publication.agent.parallelism,
    createdAt: timestamp,
    updatedAt: timestamp,
  };

  return {
    ...basePersona,
    // Catalog membership is relay-confirmed by the shared event itself. Do not
    // let a local pending toggle override this projection.
    shared: true,
    catalogSource: {
      eventId: publication.eventId,
      ownerPubkey: publication.ownerPubkey,
      isOwn,
      personaId: publication.sourcePersonaId,
    },
  };
}

export function catalogPersonasFromPublications(
  publications: readonly PersonaCatalogPublication[],
  localPersonas: readonly AgentPersona[],
  currentPubkey: string | null | undefined,
): CatalogPersona[] {
  const normalizedCurrentPubkey = currentPubkey?.toLowerCase() ?? null;
  const personas: CatalogPersona[] = [];

  for (const publication of publications) {
    const isOwn = publication.ownerPubkey === normalizedCurrentPubkey;
    personas.push(
      publicationToPersona(
        publication,
        findLocalPersonaForCatalogEntry(localPersonas, {
          ownerPubkey: publication.ownerPubkey,
          personaId: publication.sourcePersonaId,
          isOwn,
        }),
        isOwn,
      ),
    );
  }

  return personas.sort((left, right) =>
    left.displayName.localeCompare(right.displayName),
  );
}

/**
 * The local persona backing a catalog entry, if the user already has it.
 *
 * An own publication is found by id — its `d`-tag *is* the local persona id. A
 * copy of another owner's entry carries a fresh local id instead, so the only
 * link back is the `catalogSource` coordinate stored on the copy. Matching on
 * that coordinate is what stops the catalog from offering "Add" for an entry
 * the user already added, which would mint a second copy.
 */
export function findLocalPersonaForCatalogEntry(
  localPersonas: readonly AgentPersona[],
  source: CatalogSourceCoordinate & { isOwn: boolean },
): AgentPersona | undefined {
  if (source.isOwn) {
    return localPersonas.find((persona) => persona.id === source.personaId);
  }
  return localPersonas.find(
    (persona) =>
      persona.catalogSource?.ownerPubkey === source.ownerPubkey &&
      persona.catalogSource?.personaId === source.personaId,
  );
}

export function isCatalogPersona(
  persona: AgentPersona,
): persona is CatalogPersona {
  return "catalogSource" in persona && isObject(persona.catalogSource);
}
