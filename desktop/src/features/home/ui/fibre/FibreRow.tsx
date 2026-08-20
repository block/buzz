import type { Fibre } from "@/features/triage/api";
import {
  fibreArtifactCountLabel,
  fibrePeopleLabel,
  fibreSourceLabel,
  formatFibreAge,
} from "@/features/home/ui/fibre/fibreFormat";
import { fibreKindMeta } from "@/features/home/ui/fibre/fibreKinds";
import { cn } from "@/shared/lib/cn";

type FibreRowProps = {
  fibre: Fibre;
  isSelected: boolean;
  nowMs: number;
  onSelect: (id: string) => void;
};

export function FibreRow({
  fibre,
  isSelected,
  nowMs,
  onSelect,
}: FibreRowProps) {
  const kind = fibreKindMeta(fibre.kind);
  const age = formatFibreAge(
    fibre.artifacts.reduce(
      (latest, artifact) => Math.max(latest, artifact.createdAt ?? 0),
      fibre.updatedAt,
    ) || fibre.updatedAt,
    nowMs,
  );

  return (
    <button
      className={cn(
        "grid w-full grid-cols-[2.375rem_minmax(0,1fr)] gap-x-3 rounded-lg px-2.5 py-3 text-left transition-colors",
        isSelected ? "bg-muted/80" : "hover:bg-muted/40",
      )}
      data-kind={fibre.kind}
      data-testid="fibre-row"
      onClick={() => onSelect(fibre.id)}
      type="button"
    >
      <span
        className="flex h-[2.375rem] w-[2.375rem] items-center justify-center rounded-[0.625rem] text-sm font-medium tabular-nums"
        style={{ color: kind.color, background: kind.tint }}
      >
        {fibre.score}
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
          <span className="truncate">{fibrePeopleLabel(fibre.people)}</span>
        </span>
      </span>
    </button>
  );
}
