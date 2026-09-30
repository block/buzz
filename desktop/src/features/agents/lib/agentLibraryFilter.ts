import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import type { AgentAvailabilityReader } from "./useAgentAvailability";

/** Lifecycle bucket for one Agents-page card. */
export type AgentLibraryStatus = "running" | "stopped" | "not_started";

/** Status chip selection; `"all"` disables the status filter. */
export type AgentLibraryStatusFilter = "all" | AgentLibraryStatus;

export const AGENT_LIBRARY_STATUS_FILTERS: readonly {
  value: AgentLibraryStatusFilter;
  label: string;
}[] = [
  { value: "all", label: "All" },
  { value: "running", label: "Running" },
  { value: "stopped", label: "Stopped" },
  { value: "not_started", label: "Not started" },
];

export type PersonaGroup = { persona: AgentPersona; agents: ManagedAgent[] };

/** The grouped Agents library, as produced by `buildUnifiedGroups`. */
export type AgentLibraryCards = {
  groups: PersonaGroup[];
  ungrouped: ManagedAgent[];
  unknown: ManagedAgent[];
};

export type AgentLibraryFilter = {
  query: string;
  status: AgentLibraryStatusFilter;
};

export type AgentLibraryCounts = Record<AgentLibraryStatusFilter, number>;

export type AgentLibraryContext = {
  isArchived: (pubkey: string) => boolean;
  /** Relay presence, the same authority the card's availability dot reads. */
  getAvailability: AgentAvailabilityReader;
};

/**
 * Resolve the lifecycle bucket a card belongs to, following the card face:
 *
 * - A local `running` process is running.
 * - Relay presence (`online`/`away`) is running even when the local record
 *   says stopped, e.g. the same identity running under another supervisor.
 * - `deployed` is a retained receipt, not presence: after a remote shutdown
 *   the record keeps `deployed` while the relay reports offline, and the card
 *   dot shows Offline. It counts as running only until presence says offline.
 * - Any remaining non-archived instance makes the card "stopped"; a persona
 *   with no live instance has never been started.
 *
 * Archived instances are skipped so a relay-archived identity cannot keep a
 * card in "stopped" after its face renders in persona-only mode (see
 * `pickProfileAgent`).
 */
export function resolveAgentLibraryStatus(
  agents: readonly Pick<ManagedAgent, "pubkey" | "status">[],
  context: AgentLibraryContext,
): AgentLibraryStatus {
  let sawInstance = false;
  for (const agent of agents) {
    if (context.isArchived(agent.pubkey)) continue;
    const availability = context.getAvailability(agent.pubkey);
    if (
      agent.status === "running" ||
      availability === "online" ||
      availability === "away" ||
      (agent.status === "deployed" && availability === undefined)
    ) {
      return "running";
    }
    sawInstance = true;
  }
  return sawInstance ? "stopped" : "not_started";
}

/** Split a search box value into lowercase terms; every term must match. */
export function agentLibraryQueryTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

/** Case-insensitive AND match of every term against the joined fields. */
export function matchesAgentLibraryQuery(
  terms: readonly string[],
  fields: ReadonlyArray<string | null | undefined>,
): boolean {
  if (terms.length === 0) return true;
  const haystack = fields
    .filter((field): field is string => Boolean(field))
    .join("\n")
    .toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/** What the search box promises: names, descriptions, and instructions. */
function managedAgentSearchFields(
  agent: Pick<ManagedAgent, "name" | "systemPrompt">,
): Array<string | null> {
  return [agent.name, agent.systemPrompt];
}

/**
 * A persona card also searches every non-archived instance, so an
 * instance-level prompt override or renamed instance stays findable.
 */
function personaGroupSearchFields(
  group: PersonaGroup,
  isArchived: (pubkey: string) => boolean,
): Array<string | null> {
  const { persona } = group;
  return [
    persona.displayName,
    persona.description,
    persona.systemPrompt,
    ...group.agents
      .filter((agent) => !isArchived(agent.pubkey))
      .flatMap(managedAgentSearchFields),
  ];
}

/**
 * Apply the Agents-page search box and status chips to the grouped library.
 *
 * `counts` are computed over the query matches before the status chip is
 * applied, so each chip reports how many of the current search results it
 * would show. `all` equals the number of query matches.
 */
export function filterAgentLibrary(
  cards: AgentLibraryCards,
  filter: AgentLibraryFilter,
  context: AgentLibraryContext,
): AgentLibraryCards & { counts: AgentLibraryCounts } {
  const terms = agentLibraryQueryTerms(filter.query);
  const counts: AgentLibraryCounts = {
    all: 0,
    running: 0,
    stopped: 0,
    not_started: 0,
  };

  function admit(status: AgentLibraryStatus): boolean {
    counts.all += 1;
    counts[status] += 1;
    return filter.status === "all" || filter.status === status;
  }

  const groups = cards.groups.filter(
    (group) =>
      matchesAgentLibraryQuery(
        terms,
        personaGroupSearchFields(group, context.isArchived),
      ) && admit(resolveAgentLibraryStatus(group.agents, context)),
  );
  const filterStandalone = (agents: ManagedAgent[]) =>
    agents.filter(
      (agent) =>
        matchesAgentLibraryQuery(terms, managedAgentSearchFields(agent)) &&
        admit(resolveAgentLibraryStatus([agent], context)),
    );

  return {
    groups,
    ungrouped: filterStandalone(cards.ungrouped),
    unknown: filterStandalone(cards.unknown),
    counts,
  };
}

/**
 * Only an unmodified Escape clears the search box. Modified Escapes belong to
 * app shortcuts (Shift+Escape marks all channels read, see
 * `useMarkAsReadShortcuts`), which yield to any handler that called
 * `preventDefault`, so the box must not claim them.
 */
export function isPlainEscapeKey(
  event: Pick<
    KeyboardEvent,
    "key" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey"
  >,
): boolean {
  return (
    event.key === "Escape" &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey
  );
}

/** True when the toolbar is narrowing the library at all. */
export function isAgentLibraryFilterActive(
  filter: AgentLibraryFilter,
): boolean {
  return (
    filter.status !== "all" || agentLibraryQueryTerms(filter.query).length > 0
  );
}
