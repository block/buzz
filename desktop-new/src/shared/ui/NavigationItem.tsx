import { Button as BaseButton } from "@base-ui/react/button";
import type { ReactNode } from "react";

export function NavigationItem({
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
    <BaseButton
      className="navigation-item"
      data-selected={selected || undefined}
      data-inset={inset || undefined}
      onClick={onClick}
      aria-current={selected ? "page" : undefined}
    >
      {icon}
      <span className="navigation-item-label">{label}</span>
      {trailing ? (
        <span className="navigation-item-trailing">{trailing}</span>
      ) : null}
    </BaseButton>
  );
}
