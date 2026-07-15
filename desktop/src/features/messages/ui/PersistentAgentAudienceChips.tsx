import { X } from "lucide-react";

import { truncatePubkey } from "@/shared/lib/pubkey";

type PersistentAgentAudienceChipsProps = {
  getDisplayName: (pubkey: string) => string | null;
  onRemove: (pubkey: string) => void;
  pubkeys: readonly string[];
};

export function PersistentAgentAudienceChips({
  getDisplayName,
  onRemove,
  pubkeys,
}: PersistentAgentAudienceChipsProps) {
  if (pubkeys.length === 0) return null;

  return (
    <fieldset
      className="mb-2 flex flex-wrap items-center gap-1.5"
      data-testid="persistent-agent-audience"
    >
      <legend className="float-left mr-0.5 text-xs text-muted-foreground">
        Talking to
      </legend>
      {pubkeys.map((pubkey) => {
        const displayName = getDisplayName(pubkey) ?? truncatePubkey(pubkey);
        return (
          <span
            className="inline-flex max-w-48 items-center gap-1 rounded-full border border-border/60 bg-muted/60 py-1 pl-2.5 pr-1 text-xs font-medium"
            key={pubkey}
          >
            <span className="truncate">{displayName}</span>
            <button
              aria-label={`Remove ${displayName} from active audience`}
              className="inline-flex size-6 items-center justify-center rounded-full text-muted-foreground hover:bg-background hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => onRemove(pubkey)}
              type="button"
            >
              <X aria-hidden="true" className="size-3" />
            </button>
          </span>
        );
      })}
    </fieldset>
  );
}
