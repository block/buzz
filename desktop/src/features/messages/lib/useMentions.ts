import * as React from "react";

import {
  useManagedAgentsQuery,
  usePersonasQuery,
} from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { AutocompleteEdit } from "./useRichTextEditor";
import type { ChannelMember } from "@/shared/api/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";
import { trimMapToSize } from "@/shared/lib/trimMapToSize";
import { hasMention } from "./hasMention";

const MENTION_DEBOUNCE_MS = 120;

export function useMentions(
  channelId: string | null,
  externalMembers?: ChannelMember[],
  profiles?: UserProfileLookup,
) {
  const [mentionQuery, setMentionQuery] = React.useState<string | null>(null);
  const [mentionStartIndex, setMentionStartIndex] = React.useState(0);
  const [mentionSelectedIndex, setMentionSelectedIndex] = React.useState(0);
  const mentionMapRef = React.useRef<Map<string, string>>(new Map());

  const membersQuery = useChannelMembersQuery(channelId);
  const members = externalMembers ?? membersQuery.data;
  const managedAgentsQuery = useManagedAgentsQuery();
  const personasQuery = usePersonasQuery();
  const managedAgentNamesByPubkey = React.useMemo(
    () =>
      new Map(
        (managedAgentsQuery.data ?? []).map((agent) => [
          agent.pubkey.toLowerCase(),
          agent.name,
        ]),
      ),
    [managedAgentsQuery.data],
  );
  const personaNameByPubkey = React.useMemo(() => {
    const agents = managedAgentsQuery.data ?? [];
    const personas = personasQuery.data ?? [];
    const personaById = new Map(personas.map((p) => [p.id, p.displayName]));
    const lookup = new Map<string, string>();
    for (const agent of agents) {
      if (agent.personaId) {
        const name = personaById.get(agent.personaId);
        if (name) lookup.set(agent.pubkey.toLowerCase(), name);
      }
    }
    return lookup;
  }, [managedAgentsQuery.data, personasQuery.data]);

  const knownNames = React.useMemo<string[]>(() => {
    if (!members) return [];
    const names: string[] = [];
    const seen = new Set<string>();
    for (const member of members) {
      const name =
        member.displayName ??
        managedAgentNamesByPubkey.get(member.pubkey.toLowerCase());
      if (name) {
        names.push(name);
        seen.add(name.toLowerCase());
      }
      // Also include persona names so typing @Scout triggers the dropdown
      const personaName = personaNameByPubkey.get(member.pubkey.toLowerCase());
      if (personaName && !seen.has(personaName.toLowerCase())) {
        names.push(personaName);
        seen.add(personaName.toLowerCase());
      }
    }
    return names;
  }, [members, managedAgentNamesByPubkey, personaNameByPubkey]);

  /** Lower-cased version of knownNames, used for case-insensitive prefix matching. */
  const knownNamesLower = React.useMemo<string[]>(
    () => knownNames.map((n) => n.toLowerCase()),
    [knownNames],
  );

  // --- Debounce infrastructure for updateMentionQuery ---
  const debounceTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const latestValueRef = React.useRef<string>("");
  const latestCursorRef = React.useRef<number>(0);
  const knownNamesLowerRef = React.useRef<string[]>(knownNamesLower);

  // Keep the known-names ref in sync so the debounced callback never reads stale data.
  React.useEffect(() => {
    knownNamesLowerRef.current = knownNamesLower;
  }, [knownNamesLower]);

  // Clean up any pending debounce timer on unmount.
  React.useEffect(() => {
    return () => {
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, []);

  const suggestions = React.useMemo<MentionSuggestion[]>(() => {
    if (mentionQuery === null) {
      return [];
    }

    const lowerQuery = mentionQuery.toLowerCase();

    // Score a label against the query using word-boundary prefix matching.
    // Returns 0 if the full label starts with the query (best), 1 if any
    // word within the label starts with the query, or null if there's no
    // match. No arbitrary substring matching — standard for mention UX.
    const scoreLabel = (label: string): number | null => {
      const lower = label.toLowerCase();
      if (lower.startsWith(lowerQuery)) return 0;
      const words = lower.split(/[\s\-_]+/).filter(Boolean);
      if (words.some((word) => word.startsWith(lowerQuery))) return 1;
      return null;
    };

    return (members ?? [])
      .map((member) => {
        const pubkeyLower = member.pubkey.toLowerCase();

        const actualName =
          member.displayName ?? managedAgentNamesByPubkey.get(pubkeyLower);
        const personaName = personaNameByPubkey.get(pubkeyLower) ?? null;
        const label = actualName ?? member.pubkey.slice(0, 8);

        const nameScore = actualName ? scoreLabel(actualName) : null;
        const personaScore = personaName ? scoreLabel(personaName) : null;
        const labelScore =
          nameScore !== null && personaScore !== null
            ? Math.min(nameScore, personaScore)
            : (nameScore ?? personaScore);

        const pubkeyScore = pubkeyLower.startsWith(lowerQuery)
          ? 3
          : pubkeyLower.includes(lowerQuery)
            ? 4
            : null;
        const score = labelScore !== null ? labelScore : pubkeyScore;

        return { member, label, personaName, score };
      })
      .filter(
        (item): item is typeof item & { score: number } => item.score !== null,
      )
      .sort((a, b) => a.score - b.score)
      .slice(0, 8)
      .map(({ member, label, personaName }) => ({
        pubkey: member.pubkey,
        displayName: label,
        avatarUrl: profiles?.[member.pubkey.toLowerCase()]?.avatarUrl ?? null,
        role: member.role === "admin" ? "admin" : null,
        personaName,
      }));
  }, [
    managedAgentNamesByPubkey,
    members,
    mentionQuery,
    personaNameByPubkey,
    profiles,
  ]);

  const isMentionOpen = mentionQuery !== null && suggestions.length > 0;

  const insertMention = React.useCallback(
    (suggestion: MentionSuggestion, selectionEnd: number): AutocompleteEdit => {
      // Cancel any pending debounced detection — user already selected
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }

      const displayName = suggestion.displayName;
      const insertText = `@${displayName} `;

      const mentions = mentionMapRef.current;
      mentions.set(displayName, suggestion.pubkey);
      trimMapToSize(mentions, 200);
      setMentionQuery(null);
      setMentionSelectedIndex(0);

      return {
        replaceFromOffset: mentionStartIndex,
        replaceToOffset: selectionEnd,
        insertText,
      };
    },
    [mentionStartIndex],
  );

  const updateMentionQuery = React.useCallback(
    (value: string, cursorPosition: number) => {
      // Stash the latest values so the debounced callback always uses fresh data.
      latestValueRef.current = value;
      latestCursorRef.current = cursorPosition;

      // Clear any previously scheduled detection.
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }

      debounceTimerRef.current = setTimeout(() => {
        debounceTimerRef.current = null;

        const mention = detectPrefixQuery(
          "@",
          latestValueRef.current,
          latestCursorRef.current,
          knownNamesLowerRef.current,
        );
        if (mention) {
          setMentionQuery(mention.query);
          setMentionStartIndex(mention.startIndex);
          setMentionSelectedIndex(0);
        } else {
          setMentionQuery(null);
        }
      }, MENTION_DEBOUNCE_MS);
    },
    // Stable: refs are used inside the timeout, so no reactive deps needed.
    [],
  );

  const extractMentionPubkeys = React.useCallback(
    (text: string): string[] => {
      const pubkeys: string[] = [];

      for (const [displayName, pubkey] of mentionMapRef.current) {
        if (hasMention(text, displayName)) {
          pubkeys.push(pubkey);
        }
      }

      for (const member of members ?? []) {
        if (pubkeys.includes(member.pubkey)) {
          continue;
        }
        const name =
          member.displayName ??
          managedAgentNamesByPubkey.get(member.pubkey.toLowerCase());
        if (name && hasMention(text, name)) {
          pubkeys.push(member.pubkey);
        }
      }

      return [...new Set(pubkeys)];
    },
    [members, managedAgentNamesByPubkey],
  );

  const clearMentions = React.useCallback(() => {
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    mentionMapRef.current.clear();
    setMentionQuery(null);
    setMentionSelectedIndex(0);
  }, []);

  const handleMentionKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent,
    ): { handled: boolean; suggestion?: MentionSuggestion } => {
      if (!isMentionOpen) {
        return { handled: false };
      }

      if (event.key === "ArrowDown") {
        event.preventDefault();
        setMentionSelectedIndex((current) =>
          current < suggestions.length - 1 ? current + 1 : 0,
        );
        return { handled: true };
      }

      if (event.key === "ArrowUp") {
        event.preventDefault();
        setMentionSelectedIndex((current) =>
          current > 0 ? current - 1 : suggestions.length - 1,
        );
        return { handled: true };
      }

      if (
        event.key === "Tab" ||
        (event.key === "Enter" &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey &&
          !event.shiftKey)
      ) {
        event.preventDefault();
        return { handled: true, suggestion: suggestions[mentionSelectedIndex] };
      }

      if (event.key === "Escape") {
        event.preventDefault();
        setMentionQuery(null);
        return { handled: true };
      }

      return { handled: false };
    },
    [isMentionOpen, mentionSelectedIndex, suggestions],
  );

  return {
    clearMentions,
    extractMentionPubkeys,
    handleMentionKeyDown,
    insertMention,
    isMentionOpen,
    knownNames,
    mentionSelectedIndex,
    suggestions,
    updateMentionQuery,
  };
}
