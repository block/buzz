import {
  fibreArtifactCountLabel,
  fibrePeopleLabel,
  fibreSourceLabel,
  formatFibreAge,
} from "@/features/home/ui/fibre/fibreFormat";
import { fibreActivityAt } from "@/features/home/ui/fibre/fibreSort";
import {
  FIBRE_UNSEEN_DOT,
  FIBRE_UPDATED_DOT,
  fibreKindMeta,
} from "@/features/home/ui/fibre/fibreKinds";
import type { FibreDotState } from "@/features/home/ui/fibre/fibreSeen";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Fibre } from "@/features/triage/api";
import { cn } from "@/shared/lib/cn";

type FibreRowProps = {
  currentPubkey?: string;
  fibre: Fibre;
  isSelected: boolean;
  nowMs: number;
  onSelect: (id: string) => void;
  profiles?: UserProfileLookup;
  seen?: FibreDotState | null;
};

function FibreSeenDot({ state }: { state: FibreDotState }) {
  const isUpdated = state === "updated";
  return (
    <span
      aria-label={isUpdated ? "Updated" : "Unread"}
      className="h-2 w-2 shrink-0 rounded-full"
      data-state={state}
      data-testid="fibre-seen-dot"
      role="img"
      style={{ background: isUpdated ? FIBRE_UPDATED_DOT : FIBRE_UNSEEN_DOT }}
    />
  );
}

export function FibreRow({
  currentPubkey,
  fibre,
  isSelected,
  nowMs,
  onSelect,
  profiles,
  seen = null,
}: FibreRowProps) {
  const kind = fibreKindMeta(fibre.kind);
  const age = formatFibreAge(fibreActivityAt(fibre), nowMs);

  return (
    <button
      className={cn(
        "grid w-full grid-cols-[0.75rem_minmax(0,1fr)] gap-x-2 rounded-lg px-2.5 py-3 text-left transition-colors",
        isSelected ? "bg-muted/80" : "hover:bg-muted/40",
      )}
      data-kind={fibre.kind}
      data-testid="fibre-row"
      onClick={() => onSelect(fibre.id)}
      type="button"
    >
      <span className="flex h-5 items-center justify-center">
        {seen ? <FibreSeenDot state={seen} /> : null}
      </span>
      <span className="min-w-0">
        <span className="flex items-center gap-2">
          <span className="text-sm font-medium" style={{ color: kind.color }}>
            {kind.label}
          </span>
          <span className="rounded-md bg-muted px-1.5 py-px text-2xs text-muted-foreground">
            {fibreSourceLabel(fibre)}
          </span>
          <span className="ml-auto text-2xs text-muted-foreground">{age}</span>
        </span>
        <span
          className={cn(
            "mt-0.5 block text-sm leading-snug",
            isSelected ? "text-foreground" : "text-foreground/80",
          )}
        >
          {fibre.title}
        </span>
        {fibre.whyShort ? (
          <span className="mt-1 block text-xs leading-snug text-muted-foreground">
            {fibre.whyShort}
          </span>
        ) : null}
        <span className="mt-1.5 flex items-center gap-2 text-2xs text-muted-foreground">
          <span>{fibreArtifactCountLabel(fibre.artifacts.length)}</span>
          <span className="h-0.5 w-0.5 rounded-full bg-muted-foreground/50" />
          <span className="truncate">
            {fibrePeopleLabel(fibre.people, { currentPubkey, profiles })}
          </span>
        </span>
      </span>
    </button>
  );
}
