// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { cn } from "@/shared/lib/cn";

type PersonaAddedByProps = {
  className?: string;
  label?: string;
};

export function PersonaAddedBy({
  className,
  label = uiT("You"),
}: PersonaAddedByProps) {
  return (
    <p className={cn("truncate text-xs leading-tight", className)}>
      <span className="text-muted-foreground/55">{uiT("Added by")}</span>{" "}
      <span className="text-muted-foreground">{label}</span>
    </p>
  );
}
