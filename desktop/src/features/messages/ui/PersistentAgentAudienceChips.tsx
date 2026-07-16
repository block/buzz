import { Plus, X } from "lucide-react";

import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";

type PersistentAgentAudienceChipsProps = {
  getDisplayName: (pubkey: string) => string | null;
  onAdd: () => void;
  onClear: () => void;
  onRemove: (pubkey: string) => void;
  pubkeys: readonly string[];
};

export function PersistentAgentAudienceChips({
  getDisplayName,
  onAdd,
  onClear,
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
            className="inline-flex max-w-48 items-center gap-1 rounded-full border border-border bg-muted py-1 pl-2.5 pr-1 text-xs font-medium text-foreground"
            key={pubkey}
          >
            <span className="truncate">{displayName}</span>
            <Button
              aria-label={`Remove ${displayName} from active audience`}
              onClick={() => onRemove(pubkey)}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <X aria-hidden="true" />
            </Button>
          </span>
        );
      })}
      <Button
        aria-label="Mention someone"
        onClick={onAdd}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        <Plus aria-hidden="true" />
      </Button>
      {pubkeys.length > 1 ? (
        <Button onClick={onClear} size="xs" type="button" variant="ghost">
          Clear
        </Button>
      ) : null}
    </fieldset>
  );
}
