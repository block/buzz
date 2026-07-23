import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent, uploadMediaBytes } from "@/shared/api/tauri";
import {
  encodeAgentSnapshotForSend,
  type SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import type {
  AgentPersona,
  ManagedAgent,
  RelayEvent,
} from "@/shared/api/types";
import { KIND_PERSONA_CATALOG } from "@/shared/constants/kinds";

const CATALOG_FORMAT = "buzz-persona-catalog";
const CATALOG_VERSION = 1;
const MAX_CATALOG_EVENTS = 1_000;
const MAX_SNAPSHOT_JSON_BYTES = 5 * 1024 * 1024;

type CatalogStatus = "published" | "unpublished";

export type CatalogSnapshotReference = {
  url: string;
  sha256: string;
  size: number;
  type: "application/json";
  fileName: string;
};

export type CatalogPersonaShareLevel = "not-shared" | SnapshotMemoryLevel;

type CatalogAgentProjection = {
  displayName: string;
  avatarUrl: string | null;
  systemPrompt: string;
  runtime: string | null;
  model: string | null;
  provider: string | null;
};

type PublishedCatalogContent = {
  format: typeof CATALOG_FORMAT;
  version: typeof CATALOG_VERSION;
  status: "published";
  sourcePersonaId: string;
  sourceUpdatedAt: string;
  memoryLevel: SnapshotMemoryLevel;
  agent: CatalogAgentProjection;
  snapshot: CatalogSnapshotReference;
};

type UnpublishedCatalogContent = {
  format: typeof CATALOG_FORMAT;
  version: typeof CATALOG_VERSION;
  status: "unpublished";
  sourcePersonaId: string;
  sourceUpdatedAt: string;
};

type CatalogContent = PublishedCatalogContent | UnpublishedCatalogContent;

export type PersonaCatalogPublication = {
  eventId: string;
  ownerPubkey: string;
  sourcePersonaId: string;
  sourceUpdatedAt: string;
  createdAt: number;
  status: CatalogStatus;
  memoryLevel: SnapshotMemoryLevel | null;
  agent: CatalogAgentProjection | null;
  snapshot: CatalogSnapshotReference | null;
};

export type CatalogPersona = AgentPersona & {
  catalogSource: {
    eventId: string;
    ownerPubkey: string;
    isOwn: boolean;
    sourcePersonaId: string;
    sourceUpdatedAt: string;
    memoryLevel: SnapshotMemoryLevel;
    snapshot: CatalogSnapshotReference;
  };
};

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringOrNull(value: unknown): string | null | undefined {
  return typeof value === "string" || value === null ? value : undefined;
}

function extractTag(event: RelayEvent, name: string): string | null {
  const matches = event.tags.filter(
    (tag) => tag[0] === name && typeof tag[1] === "string",
  );
  return matches.length === 1 ? (matches[0]?.[1] ?? null) : null;
}

function isMemoryLevel(value: unknown): value is SnapshotMemoryLevel {
  return value === "none" || value === "core" || value === "everything";
}

function isSafeSnapshotReference(
  value: unknown,
): value is CatalogSnapshotReference {
  if (!isObject(value)) return false;
  const fileName = value.fileName;
  const sha256 = value.sha256;
  const size = value.size;
  const url = value.url;
  return (
    typeof url === "string" &&
    url.length > 0 &&
    !/[\s()]/u.test(url) &&
    (() => {
      try {
        const parsed = new URL(url);
        return parsed.protocol === "https:" || parsed.protocol === "http:";
      } catch {
        return false;
      }
    })() &&
    typeof sha256 === "string" &&
    /^[0-9a-f]{64}$/u.test(sha256) &&
    typeof size === "number" &&
    Number.isSafeInteger(size) &&
    size > 0 &&
    size <= MAX_SNAPSHOT_JSON_BYTES &&
    value.type === "application/json" &&
    typeof fileName === "string" &&
    fileName.toLowerCase().endsWith(".agent.json") &&
    !fileName.includes("/") &&
    !fileName.includes("\\")
  );
}

function parseCatalogContent(event: RelayEvent): CatalogContent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(event.content);
  } catch {
    return null;
  }
  if (!isObject(parsed)) return null;

  const sourcePersonaId = extractTag(event, "d");
  const status = extractTag(event, "status");
  const sourceUpdatedAt = extractTag(event, "source_updated_at");
  if (
    event.kind !== KIND_PERSONA_CATALOG ||
    parsed.format !== CATALOG_FORMAT ||
    parsed.version !== CATALOG_VERSION ||
    parsed.status !== status ||
    parsed.sourcePersonaId !== sourcePersonaId ||
    parsed.sourceUpdatedAt !== sourceUpdatedAt ||
    !sourcePersonaId ||
    !sourceUpdatedAt ||
    (status !== "published" && status !== "unpublished")
  ) {
    return null;
  }

  if (status === "unpublished") {
    return {
      format: CATALOG_FORMAT,
      version: CATALOG_VERSION,
      status,
      sourcePersonaId,
      sourceUpdatedAt,
    };
  }

  const memoryLevel = extractTag(event, "memory");
  if (
    !isMemoryLevel(memoryLevel) ||
    parsed.memoryLevel !== memoryLevel ||
    !isObject(parsed.agent) ||
    typeof parsed.agent.displayName !== "string" ||
    parsed.agent.displayName.trim().length === 0 ||
    typeof parsed.agent.systemPrompt !== "string" ||
    stringOrNull(parsed.agent.avatarUrl) === undefined ||
    stringOrNull(parsed.agent.runtime) === undefined ||
    stringOrNull(parsed.agent.model) === undefined ||
    stringOrNull(parsed.agent.provider) === undefined ||
    !isSafeSnapshotReference(parsed.snapshot)
  ) {
    return null;
  }

  return {
    format: CATALOG_FORMAT,
    version: CATALOG_VERSION,
    status,
    sourcePersonaId,
    sourceUpdatedAt,
    memoryLevel,
    agent: {
      displayName: parsed.agent.displayName,
      avatarUrl: parsed.agent.avatarUrl as string | null,
      systemPrompt: parsed.agent.systemPrompt,
      runtime: parsed.agent.runtime as string | null,
      model: parsed.agent.model as string | null,
      provider: parsed.agent.provider as string | null,
    },
    snapshot: parsed.snapshot,
  };
}

/**
 * Collapse relay results to one latest head per `(author, sourcePersonaId)`.
 * The relay already applies NIP-33 replacement, but doing this client-side
 * makes discovery deterministic against older relays and test fixtures.
 */
export function catalogPublicationsFromEvents(
  events: readonly RelayEvent[],
): PersonaCatalogPublication[] {
  const sorted = [...events].sort(
    (left, right) =>
      right.created_at - left.created_at || right.id.localeCompare(left.id),
  );
  const seenCoordinates = new Set<string>();
  const publications: PersonaCatalogPublication[] = [];

  for (const event of sorted) {
    const sourcePersonaId = extractTag(event, "d");
    if (!sourcePersonaId) continue;
    const coordinate = `${event.pubkey.toLowerCase()}:${sourcePersonaId}`;
    if (seenCoordinates.has(coordinate)) continue;
    seenCoordinates.add(coordinate);

    const content = parseCatalogContent(event);
    if (!content) continue;
    publications.push({
      eventId: event.id,
      ownerPubkey: event.pubkey.toLowerCase(),
      sourcePersonaId: content.sourcePersonaId,
      sourceUpdatedAt: content.sourceUpdatedAt,
      createdAt: event.created_at,
      status: content.status,
      memoryLevel: content.status === "published" ? content.memoryLevel : null,
      agent: content.status === "published" ? content.agent : null,
      snapshot: content.status === "published" ? content.snapshot : null,
    });
  }

  return publications;
}

export async function fetchPersonaCatalogPublications(): Promise<
  PersonaCatalogPublication[]
> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_PERSONA_CATALOG],
    limit: MAX_CATALOG_EVENTS,
  });
  return catalogPublicationsFromEvents(events);
}

export function catalogPersonasFromPublications(
  publications: readonly PersonaCatalogPublication[],
  localPersonas: readonly AgentPersona[],
  currentPubkey: string | null | undefined,
): CatalogPersona[] {
  const normalizedCurrentPubkey = currentPubkey?.toLowerCase() ?? null;
  return publications
    .filter(
      (
        publication,
      ): publication is PersonaCatalogPublication & {
        status: "published";
        memoryLevel: SnapshotMemoryLevel;
        agent: CatalogAgentProjection;
        snapshot: CatalogSnapshotReference;
      } =>
        publication.status === "published" &&
        publication.memoryLevel !== null &&
        publication.agent !== null &&
        publication.snapshot !== null,
    )
    .map((publication) => {
      const ownLocalPersona =
        publication.ownerPubkey === normalizedCurrentPubkey
          ? localPersonas.find(
              (persona) => persona.id === publication.sourcePersonaId,
            )
          : undefined;
      const persona: CatalogPersona = {
        id:
          publication.ownerPubkey === normalizedCurrentPubkey
            ? publication.sourcePersonaId
            : `catalog:${publication.ownerPubkey}:${publication.sourcePersonaId}`,
        displayName: publication.agent.displayName,
        avatarUrl: publication.agent.avatarUrl,
        systemPrompt: publication.agent.systemPrompt,
        runtime: publication.agent.runtime,
        model: publication.agent.model,
        provider: publication.agent.provider,
        namePool: [],
        isBuiltIn: false,
        isActive: ownLocalPersona?.isActive ?? false,
        sourceTeam: null,
        envVars: {},
        respondTo: null,
        respondToAllowlist: [],
        parallelism: null,
        createdAt: publication.sourceUpdatedAt,
        updatedAt: publication.sourceUpdatedAt,
        catalogSource: {
          eventId: publication.eventId,
          ownerPubkey: publication.ownerPubkey,
          isOwn: publication.ownerPubkey === normalizedCurrentPubkey,
          sourcePersonaId: publication.sourcePersonaId,
          sourceUpdatedAt: publication.sourceUpdatedAt,
          memoryLevel: publication.memoryLevel,
          snapshot: publication.snapshot,
        },
      };
      return persona;
    })
    .sort((left, right) => left.displayName.localeCompare(right.displayName));
}

export function isCatalogPersona(
  persona: AgentPersona,
): persona is CatalogPersona {
  return "catalogSource" in persona && isObject(persona.catalogSource);
}

export function ownCatalogPublication(
  publications: readonly PersonaCatalogPublication[],
  ownerPubkey: string | null | undefined,
  sourcePersonaId: string,
): PersonaCatalogPublication | null {
  if (!ownerPubkey) return null;
  const normalizedOwner = ownerPubkey.toLowerCase();
  return (
    publications.find(
      (publication) =>
        publication.ownerPubkey === normalizedOwner &&
        publication.sourcePersonaId === sourcePersonaId,
    ) ?? null
  );
}

function copyOptionalString(
  source: JsonObject,
  target: JsonObject,
  key: string,
): void {
  if (typeof source[key] === "string") target[key] = source[key];
}

function copyOptionalNumber(
  source: JsonObject,
  target: JsonObject,
  key: string,
): void {
  if (
    typeof source[key] === "number" &&
    Number.isFinite(source[key]) &&
    source[key] >= 0
  ) {
    target[key] = source[key];
  }
}

/**
 * Rebuild a catalog snapshot from a strict public allowlist.
 *
 * The normal portable snapshot already excludes identity keys, auth tags,
 * environment variables, relay URLs, commands, and runtime state. Catalog
 * publication applies a second boundary: response allowlists are also removed
 * because they disclose source-community pubkeys and are not portable.
 */
export function sanitizeCatalogSnapshotBytes(
  fileBytes: readonly number[],
  expectedMemoryLevel: SnapshotMemoryLevel,
): number[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder().decode(Uint8Array.from(fileBytes)));
  } catch {
    throw new Error("Couldn’t prepare this agent for the community catalog.");
  }
  if (
    !isObject(parsed) ||
    parsed.format !== "buzz-agent-snapshot" ||
    parsed.version !== 1 ||
    !isObject(parsed.definition) ||
    !isObject(parsed.profile) ||
    !isObject(parsed.memory) ||
    parsed.memory.level !== expectedMemoryLevel ||
    !Array.isArray(parsed.memory.entries)
  ) {
    throw new Error("The generated agent snapshot is invalid.");
  }

  const definition: JsonObject = {};
  copyOptionalString(parsed.definition, definition, "name");
  if (typeof parsed.definition.sourceIsBuiltIn === "boolean") {
    definition.sourceIsBuiltIn = parsed.definition.sourceIsBuiltIn;
  }
  for (const key of [
    "systemPrompt",
    "runtime",
    "model",
    "provider",
    "respondTo",
  ]) {
    copyOptionalString(parsed.definition, definition, key);
  }
  for (const key of [
    "parallelism",
    "idleTimeoutSeconds",
    "maxTurnDurationSeconds",
  ]) {
    copyOptionalNumber(parsed.definition, definition, key);
  }
  if (
    Array.isArray(parsed.definition.namePool) &&
    parsed.definition.namePool.every((value) => typeof value === "string")
  ) {
    definition.namePool = [...parsed.definition.namePool];
  }

  const profile: JsonObject = {};
  for (const key of ["displayName", "about", "avatarDataUrl", "avatarUrl"]) {
    copyOptionalString(parsed.profile, profile, key);
  }
  if (
    typeof profile.displayName !== "string" ||
    profile.displayName.length === 0
  ) {
    throw new Error("The generated agent snapshot has no display name.");
  }

  const entries = parsed.memory.entries.map((entry) => {
    if (
      !isObject(entry) ||
      typeof entry.slug !== "string" ||
      entry.slug.length === 0 ||
      typeof entry.body !== "string"
    ) {
      throw new Error("The generated agent snapshot has invalid memory.");
    }
    return { slug: entry.slug, body: entry.body };
  });
  const duplicateSlugs =
    new Set(entries.map((entry) => entry.slug)).size !== entries.length;
  const invalidMemorySelection =
    duplicateSlugs ||
    (expectedMemoryLevel === "none" && entries.length > 0) ||
    (expectedMemoryLevel === "core" &&
      (entries.length > 1 || entries.some((entry) => entry.slug !== "core"))) ||
    (expectedMemoryLevel === "everything" &&
      entries.some(
        (entry) => entry.slug !== "core" && !entry.slug.startsWith("mem/"),
      ));
  if (invalidMemorySelection) {
    throw new Error(
      "The generated agent snapshot includes memory outside the selected sharing level.",
    );
  }

  const sanitized = {
    format: "buzz-agent-snapshot",
    version: 1,
    definition,
    profile,
    memory: {
      level: expectedMemoryLevel,
      entries,
    },
  };
  const bytes = Array.from(new TextEncoder().encode(JSON.stringify(sanitized)));
  if (bytes.length > MAX_SNAPSHOT_JSON_BYTES) {
    throw new Error(
      "This agent is too large for the community catalog. Share less memory and try again.",
    );
  }
  return bytes;
}

function publicAvatarUrl(avatarUrl: string | null): string | null {
  if (!avatarUrl || avatarUrl.startsWith("data:") || avatarUrl.length > 2_048) {
    return null;
  }
  return avatarUrl;
}

function monotonicCreatedAt(previousCreatedAt?: number | null): number {
  return Math.max(Math.floor(Date.now() / 1_000), (previousCreatedAt ?? 0) + 1);
}

function requirePublishedCatalogEvent(
  event: RelayEvent,
): PersonaCatalogPublication {
  const publication = catalogPublicationsFromEvents([event])[0];
  if (!publication) {
    throw new Error("The relay returned an invalid catalog publication.");
  }
  return publication;
}

export async function publishPersonaToCatalog(input: {
  persona: AgentPersona;
  memoryLevel: SnapshotMemoryLevel;
  linkedAgentPubkey: string | null;
  previousCreatedAt?: number | null;
}): Promise<PersonaCatalogPublication> {
  if (input.persona.isBuiltIn) {
    throw new Error("Built-in agents can’t be published to the catalog.");
  }
  if (input.memoryLevel !== "none" && !input.linkedAgentPubkey) {
    throw new Error("Start this agent before sharing its memory.");
  }

  const encoded = await encodeAgentSnapshotForSend(
    input.persona.id,
    input.memoryLevel,
    "json",
    input.linkedAgentPubkey,
  );
  const snapshotBytes = sanitizeCatalogSnapshotBytes(
    encoded.fileBytes,
    input.memoryLevel,
  );
  const descriptor = await uploadMediaBytes(snapshotBytes, encoded.fileName);
  if (
    !/^[0-9a-f]{64}$/u.test(descriptor.sha256) ||
    descriptor.size !== snapshotBytes.length ||
    !descriptor.url
  ) {
    throw new Error("The relay returned an invalid catalog snapshot receipt.");
  }

  const content: PublishedCatalogContent = {
    format: CATALOG_FORMAT,
    version: CATALOG_VERSION,
    status: "published",
    sourcePersonaId: input.persona.id,
    sourceUpdatedAt: input.persona.updatedAt,
    memoryLevel: input.memoryLevel,
    agent: {
      displayName: input.persona.displayName,
      avatarUrl: publicAvatarUrl(input.persona.avatarUrl),
      systemPrompt: input.persona.systemPrompt,
      runtime: input.persona.runtime,
      model: input.persona.model,
      provider: input.persona.provider,
    },
    snapshot: {
      url: descriptor.url,
      sha256: descriptor.sha256,
      size: descriptor.size,
      type: "application/json",
      fileName: encoded.fileName,
    },
  };
  const event = await signRelayEvent({
    kind: KIND_PERSONA_CATALOG,
    content: JSON.stringify(content),
    createdAt: monotonicCreatedAt(input.previousCreatedAt),
    tags: [
      ["d", input.persona.id],
      ["status", "published"],
      ["source_updated_at", input.persona.updatedAt],
      ["memory", input.memoryLevel],
    ],
  });
  const published = await relayClient.publishEvent(
    event,
    "Timed out publishing this agent to the catalog.",
    "Failed to publish this agent to the catalog.",
  );
  return requirePublishedCatalogEvent(published);
}

export async function unpublishPersonaFromCatalog(input: {
  persona: AgentPersona;
  previousCreatedAt?: number | null;
}): Promise<PersonaCatalogPublication> {
  const content: UnpublishedCatalogContent = {
    format: CATALOG_FORMAT,
    version: CATALOG_VERSION,
    status: "unpublished",
    sourcePersonaId: input.persona.id,
    sourceUpdatedAt: input.persona.updatedAt,
  };
  const event = await signRelayEvent({
    kind: KIND_PERSONA_CATALOG,
    content: JSON.stringify(content),
    createdAt: monotonicCreatedAt(input.previousCreatedAt),
    tags: [
      ["d", input.persona.id],
      ["status", "unpublished"],
      ["source_updated_at", input.persona.updatedAt],
    ],
  });
  const published = await relayClient.publishEvent(
    event,
    "Timed out removing this agent from the catalog.",
    "Failed to remove this agent from the catalog.",
  );
  return requirePublishedCatalogEvent(published);
}

export function linkedAgentPubkeyForPersona(
  personaId: string,
  managedAgents: readonly ManagedAgent[],
): string | null {
  return (
    managedAgents.find((agent) => agent.personaId === personaId)?.pubkey ?? null
  );
}
