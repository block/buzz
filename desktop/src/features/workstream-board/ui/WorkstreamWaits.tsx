import type { WorkstreamWait } from "@/features/workstream-board/lib/workstreamWaits";
export function WorkstreamWaits({
  waits,
}: {
  waits: readonly WorkstreamWait[];
}) {
  if (waits.length === 0) return null;
  return (
    <section
      aria-label="Waiting on"
      className="pointer-events-auto mt-4 space-y-1.5"
      data-testid="workstream-waits"
    >
      <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        Waiting on
      </p>
      <ul className="space-y-1">
        {waits.map((wait) => (
          <li
            className="rounded-md border border-amber-500/25 bg-amber-500/10 px-2 py-1.5 text-2xs text-foreground"
            data-testid={`workstream-wait-${wait.key}`}
            key={wait.key}
          >
            <span className="font-medium">{wait.actor.name}</span>
            {" · "}
            {wait.reason}
          </li>
        ))}
      </ul>
    </section>
  );
}
