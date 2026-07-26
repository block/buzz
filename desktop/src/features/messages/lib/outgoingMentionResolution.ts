import type { MentionCandidate } from "./mentionCandidates";
import { resolveMentionedNames } from "./hasMention";
import type { AgentPersona } from "@/shared/api/types";

export type PersonaMentionTarget = {
  displayName: string;
  persona: AgentPersona;
};

type OutgoingMentionPubkeyOptions = {
  candidates: readonly Pick<
    MentionCandidate,
    "displayName" | "isMember" | "pubkey"
  >[];
  selectedMentions: ReadonlyMap<string, string>;
  selectedPersonaMentions: ReadonlyMap<string, string>;
  text: string;
};

export function resolveOutgoingMentionPubkeys({
  candidates,
  selectedMentions,
  selectedPersonaMentions,
  text,
}: OutgoingMentionPubkeyOptions): string[] {
  const pubkeys: string[] = [];
  const selectedNames = [
    ...selectedMentions.keys(),
    ...selectedPersonaMentions.keys(),
  ];
  const selectedDisplayNames = new Set(
    selectedNames.map((name) => name.trim().toLowerCase()),
  );
  const resolvedDisplayNames = new Set(
    resolveMentionedNames(
      text,
      [
        ...selectedNames,
        ...candidates.flatMap((candidate) =>
          candidate.displayName ? [candidate.displayName] : [],
        ),
      ],
      selectedNames,
    ).map((name) => name.toLowerCase()),
  );

  for (const [displayName, pubkey] of selectedMentions) {
    if (resolvedDisplayNames.has(displayName.trim().toLowerCase())) {
      pubkeys.push(pubkey);
    }
  }

  for (const candidate of candidates) {
    const name = candidate.displayName;
    if (
      !candidate.pubkey ||
      !candidate.isMember ||
      pubkeys.includes(candidate.pubkey) ||
      !name ||
      selectedDisplayNames.has(name.trim().toLowerCase()) ||
      !resolvedDisplayNames.has(name.trim().toLowerCase())
    ) {
      continue;
    }
    pubkeys.push(candidate.pubkey);
  }

  return [...new Set(pubkeys)];
}

type OutgoingMentionPersonaOptions = {
  activePersonaById: ReadonlyMap<string, AgentPersona>;
  selectedPersonaMentions: ReadonlyMap<string, string>;
  text: string;
};

export function resolveOutgoingMentionPersonas({
  activePersonaById,
  selectedPersonaMentions,
  text,
}: OutgoingMentionPersonaOptions): PersonaMentionTarget[] {
  const targets: PersonaMentionTarget[] = [];
  const seen = new Set<string>();
  const displayNames = [...selectedPersonaMentions.keys()];
  const resolvedDisplayNames = new Set(
    resolveMentionedNames(text, displayNames, displayNames).map((name) =>
      name.toLowerCase(),
    ),
  );

  for (const [displayName, personaId] of selectedPersonaMentions) {
    if (
      seen.has(personaId) ||
      !resolvedDisplayNames.has(displayName.trim().toLowerCase())
    ) {
      continue;
    }

    const persona = activePersonaById.get(personaId);
    if (!persona) continue;

    targets.push({ displayName, persona });
    seen.add(personaId);
  }

  return targets;
}
