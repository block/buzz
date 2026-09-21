import { useTranslation } from "@/i18n";
import { cn } from "@/shared/lib/cn";

type PersonaAddedByProps = {
  className?: string;
  label?: string;
};

export function PersonaAddedBy({ className, label }: PersonaAddedByProps) {
  const { t } = useTranslation();
  return (
    <p className={cn("truncate text-xs leading-tight", className)}>
      <span className="text-muted-foreground/55">
        {t("agents.persona-added-by.label")}
      </span>{" "}
      <span className="text-muted-foreground">
        {label ?? t("messages.drafts.you")}
      </span>
    </p>
  );
}
