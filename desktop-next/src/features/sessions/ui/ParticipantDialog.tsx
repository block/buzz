import { Dialog } from "@base-ui-components/react/dialog";
import { IconPlus, IconSearch, IconUsers, IconX } from "@tabler/icons-react";
import { useRef, useState } from "react";
import { runtime } from "@/shared/runtime/client";
import type { Participant } from "../types";

type SearchPerson = {
  pubkey: string;
  display_name?: string;
  is_agent?: boolean;
};

export function ParticipantDialog({
  channelId,
  participants,
  onChanged,
}: {
  channelId: string;
  participants: Participant[];
  onChanged: () => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchPerson[]>([]);
  const [status, setStatus] = useState<string | null>(null);
  const searchGeneration = useRef(0);

  async function search(value: string) {
    setQuery(value);
    const generation = ++searchGeneration.current;
    if (!value.trim()) {
      setResults([]);
      setStatus(null);
      return;
    }
    setStatus("Searching…");
    try {
      const people = (await runtime.searchPeople(value)) as SearchPerson[];
      if (generation !== searchGeneration.current) return;
      const existing = new Set(
        participants.map((participant) => participant.pubkey),
      );
      const available = people.filter((person) => !existing.has(person.pubkey));
      setResults(available);
      setStatus(available.length ? null : "No matching people or agents.");
    } catch {
      if (generation === searchGeneration.current) {
        setResults([]);
        setStatus("Search failed. Try again.");
      }
    }
  }

  async function add(person: SearchPerson) {
    setStatus(`Adding ${person.display_name ?? "participant"}…`);
    try {
      const outcome = await runtime.addParticipant(
        channelId,
        person.pubkey,
        Boolean(person.is_agent),
      );
      if (outcome.errors.length > 0) {
        setStatus("Could not add this participant. Try again.");
        return;
      }
      setStatus(`${person.display_name ?? "Participant"} joined`);
      setResults((current) =>
        current.filter((candidate) => candidate.pubkey !== person.pubkey),
      );
      await onChanged();
    } catch {
      setStatus("Could not add this participant. Try again.");
    }
  }

  return (
    <Dialog.Root>
      <Dialog.Trigger className="participant-trigger">
        <IconUsers size={16} stroke={1.6} aria-hidden="true" />
        <span>
          {participants.length <= 1
            ? "Only you"
            : `${participants.length} participants`}
        </span>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Backdrop className="dialog-backdrop" />
        <Dialog.Popup className="dialog-popup">
          <header className="dialog-heading">
            <div>
              <Dialog.Title className="text-heading text-primary">
                People and agents
              </Dialog.Title>
              <Dialog.Description className="mt-1 text-body-sm text-secondary">
                New participants can read the existing Session history.
              </Dialog.Description>
            </div>
            <Dialog.Close
              className="icon-button"
              aria-label="Close participant dialog"
            >
              <IconX size={18} stroke={1.6} aria-hidden="true" />
            </Dialog.Close>
          </header>
          <div className="participant-list">
            {participants.map((participant) => (
              <div className="participant-row" key={participant.pubkey}>
                <span className="participant-avatar" aria-hidden="true">
                  {(participant.displayName ?? "?").slice(0, 1)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-body text-primary">
                    {participant.displayName ?? "Unknown participant"}
                  </span>
                  <span className="block text-body-sm text-tertiary">
                    {participant.isAgent ? "Agent" : participant.role}
                  </span>
                </span>
              </div>
            ))}
          </div>
          <label className="participant-search">
            <IconSearch size={17} stroke={1.6} aria-hidden="true" />
            <span className="sr-only">Find a person or agent</span>
            <input
              value={query}
              onChange={(event) => void search(event.target.value)}
              placeholder="Find a person or agent"
            />
          </label>
          {results.length > 0 ? (
            <fieldset className="search-results">
              <legend className="sr-only">Search results</legend>
              {results.map((person) => (
                <button
                  type="button"
                  key={person.pubkey}
                  onClick={() => void add(person)}
                >
                  <span>{person.display_name ?? "Unknown identity"}</span>
                  <span className="text-body-sm text-tertiary">
                    {person.is_agent ? "Agent" : "Person"}
                  </span>
                  <IconPlus size={16} stroke={1.6} aria-hidden="true" />
                </button>
              ))}
            </fieldset>
          ) : null}
          {status ? (
            <p className="text-body-sm text-secondary" role="status">
              {status}
            </p>
          ) : null}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
