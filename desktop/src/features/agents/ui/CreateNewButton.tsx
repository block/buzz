import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/shared/ui/button";

type CreateNewButtonProps = {
  ariaLabel?: string;
  disabled?: boolean;
  label?: string;
  onClick: () => void;
  variant?: "default" | "outline";
};

export function CreateNewButton({
  ariaLabel,
  disabled = false,
  label,
  onClick,
  variant = "default",
}: CreateNewButtonProps) {
  const { t } = useTranslation();
  return (
    <Button
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={onClick}
      size="sm"
      type="button"
      variant={variant}
    >
      <Plus className="h-4 w-4" />
      {label ?? t("common.new")}
    </Button>
  );
}
