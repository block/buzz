import * as React from "react";
import { verifyEvent } from "nostr-tools/pure";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_IA_ARCHIVE_REQUEST,
  KIND_MANAGED_AGENT,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

type ManagedAgentEventContent = {
  persona_id?: unknown;
};

function eventHasValidSignature(event: RelayEvent): boolean {
  try {
    return verifyEvent({
      id: event.id,
      pubkey: event.pubkey,
      created_at: event.created_at,
      kind: event.kind,
      tags: event.tags,
      content: event.content,
      sig: event.sig,
    });
  } catch {
    return false;
  }
}

function personaIdFromEventContent(event: RelayEvent): string | null {
  try {
    const content = JSON.parse(event.content) as ManagedAgentEventContent;
    return typeof content.persona_id === "string" &&
      content.persona_id.trim().length > 0
      ? content.persona_id
      : null;
  } catch {
    return null;
  }
}

export function personaIdFromOwnedManagedAgentEvent(
  event: RelayEvent | null,
  ownerPubkey: string,
  agentPubkey: string,
): string | null {
  if (!event || event.kind !== KIND_MANAGED_AGENT) return null;

  const owner = normalizePubkey(ownerPubkey);
  const agent = normalizePubkey(agentPubkey);
  if (!owner || !agent || normalizePubkey(event.pubkey) !== owner) return null;
  if (
    !event.tags.some(
      (tag) => tag[0] === "d" && normalizePubkey(tag[1] ?? "") === agent,
    )
  ) {
    return null;
  }
  if (!eventHasValidSignature(event)) return null;

  return personaIdFromEventContent(event);
}

export function personaIdFromOwnedManagedAgentArchive(
  event: RelayEvent | null,
  ownerPubkey: string,
  agentPubkey: string,
): string | null {
  if (!event || event.kind !== KIND_IA_ARCHIVE_REQUEST) return null;

  const owner = normalizePubkey(ownerPubkey);
  const agent = normalizePubkey(agentPubkey);
  if (!owner || !agent || normalizePubkey(event.pubkey) !== owner) return null;
  if (
    !event.tags.some(
      (tag) => tag[0] === "p" && normalizePubkey(tag[1] ?? "") === agent,
    )
  ) {
    return null;
  }
  if (!eventHasValidSignature(event)) return null;

  return personaIdFromEventContent(event);
}

type PersonaLookupResult = {
  key: string;
  personaId: string | null;
};

/**
 * Resolve an owned historical agent key back to its persona. The live
 * kind:30177 projection provides the alias before deletion; the owner-signed
 * NIP-IA archive request preserves it after the projection is tombstoned.
 */
export function useOwnedManagedAgentPersonaId(input: {
  agentPubkey: string | undefined;
  enabled: boolean;
  ownerPubkey: string | undefined;
}): string | null {
  const { agentPubkey, enabled, ownerPubkey } = input;
  const [result, setResult] = React.useState<PersonaLookupResult | null>(null);
  const normalizedOwner = normalizePubkey(ownerPubkey ?? "");
  const normalizedAgent = normalizePubkey(agentPubkey ?? "");
  const lookupKey =
    enabled && normalizedOwner && normalizedAgent
      ? `${normalizedOwner}:${normalizedAgent}`
      : null;

  React.useEffect(() => {
    let cancelled = false;

    if (!lookupKey) {
      return () => {
        cancelled = true;
      };
    }

    void Promise.allSettled([
      relayClient.fetchFirstEvent({
        kinds: [KIND_MANAGED_AGENT],
        authors: [normalizedOwner],
        "#d": [normalizedAgent],
        limit: 1,
      }),
      relayClient.fetchFirstEvent({
        kinds: [KIND_IA_ARCHIVE_REQUEST],
        authors: [normalizedOwner],
        "#p": [normalizedAgent],
        limit: 1,
      }),
    ]).then(([managedAgentResult, archiveResult]) => {
      if (cancelled) return;
      const managedAgentEvent =
        managedAgentResult.status === "fulfilled"
          ? managedAgentResult.value
          : null;
      const archiveEvent =
        archiveResult.status === "fulfilled" ? archiveResult.value : null;
      setResult({
        key: lookupKey,
        personaId:
          personaIdFromOwnedManagedAgentEvent(
            managedAgentEvent,
            normalizedOwner,
            normalizedAgent,
          ) ??
          personaIdFromOwnedManagedAgentArchive(
            archiveEvent,
            normalizedOwner,
            normalizedAgent,
          ),
      });
    });

    return () => {
      cancelled = true;
    };
  }, [lookupKey, normalizedAgent, normalizedOwner]);

  return result?.key === lookupKey ? result.personaId : null;
}
