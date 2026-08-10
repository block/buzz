import { cn } from "@/shared/lib/cn";

type PersonaAddedByProps = {
  className?: string;
  label?: string;
};

export function PersonaAddedBy({
  className,
  label = "You",
}: PersonaAddedByProps) {
  return (
    <p className={cn("truncate text-xs leading-tight", className)}>
      <span className="text-muted-foreground/55">Added by</span>{" "}
      <span className="text-muted-foreground">{label}</span>
    </p>
  );
}
