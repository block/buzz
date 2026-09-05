import type { ReactNode } from "react";
import { Button } from "./Button";

export function NavigatorRow({
  label,
  icon,
  trailing,
  selected = false,
  inset = false,
  onClick,
}: {
  label: string;
  icon?: ReactNode;
  trailing?: ReactNode;
  selected?: boolean;
  inset?: boolean;
  onClick?: () => void;
}) {
  return (
    <Button
      variant="ghost"
      size="compact"
      data-navigator-row=""
      data-selected={selected || undefined}
      data-inset={inset || undefined}
      onClick={onClick}
      aria-current={selected ? "page" : undefined}
    >
      {icon}
      <span className="navigator-row-label">{label}</span>
      {trailing ? (
        <span className="navigator-row-trailing">{trailing}</span>
      ) : null}
    </Button>
  );
}
