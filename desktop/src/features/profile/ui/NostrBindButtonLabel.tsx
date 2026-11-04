import { cn } from "@/shared/lib/cn";

const labelClass =
  "col-start-1 row-start-1 transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:translate-y-0 motion-reduce:duration-0";

/** Animate binding action labels using operation state, independent of language. */
export function NostrBindButtonLabel({
  active,
  idleLabel,
  activeLabel,
}: {
  active: boolean;
  idleLabel: string;
  activeLabel: string;
}) {
  return (
    <span
      aria-hidden="true"
      className="inline-grid h-5 place-items-center overflow-hidden"
    >
      <span
        className={cn(
          labelClass,
          !active ? "translate-y-0 opacity-100" : "-translate-y-0.5 opacity-0",
        )}
      >
        {idleLabel}
      </span>
      <span
        className={cn(
          labelClass,
          active ? "translate-y-0 opacity-100" : "translate-y-0.5 opacity-0",
        )}
      >
        {activeLabel}
      </span>
    </span>
  );
}
