import { X } from "lucide-react";

export type ComposerAudienceChip = {
  displayName: string;
  pubkey: string;
};

export function ComposerAudienceChips({
  audience,
  onRemove,
}: {
  audience: readonly ComposerAudienceChip[];
  onRemove: (pubkey: string) => void;
}) {
  if (audience.length === 0) return null;

  return (
    <div
      className="mb-2 flex flex-wrap gap-1.5"
      data-testid="composer-audience-chips"
    >
      {audience.map(({ displayName, pubkey }) => (
        <span
          className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-1 text-2xs text-foreground"
          data-testid={`composer-audience-chip-${pubkey}`}
          key={pubkey}
        >
          {displayName}
          <button
            aria-label={`Remove ${displayName}`}
            className="rounded-full text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            data-testid={`composer-audience-chip-remove-${pubkey}`}
            onClick={() => onRemove(pubkey)}
            type="button"
          >
            <X aria-hidden="true" className="size-3" />
          </button>
        </span>
      ))}
    </div>
  );
}
