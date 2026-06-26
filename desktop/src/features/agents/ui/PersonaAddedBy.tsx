import { cn } from "@/shared/lib/cn";

type PersonaAddedByProps = {
  className?: string;
};

export function PersonaAddedBy({ className }: PersonaAddedByProps) {
  return (
    <p className={cn("truncate text-xs leading-tight", className)}>
      <span className="text-muted-foreground/55">Added by</span>{" "}
      <span className="text-muted-foreground">You</span>
    </p>
  );
}
