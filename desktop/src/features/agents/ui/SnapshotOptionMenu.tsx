import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export type SnapshotOption = {
  value: string;
  label: string;
};

export function SnapshotOptionMenu({
  ariaLabel,
  className,
  disabled = false,
  onOpenChange,
  options,
  testId,
  value,
  onValueChange,
}: {
  ariaLabel: string;
  className?: string;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
  options: readonly SnapshotOption[];
  testId: string;
  value: string;
  onValueChange: (value: string) => void;
}) {
  const selectedLabel =
    options.find((option) => option.value === value)?.label ?? "";

  return (
    <DropdownMenu modal={false} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={ariaLabel}
          className={cn(
            "inline-flex h-8 w-auto shrink-0 items-center justify-end gap-1.5 rounded-md bg-transparent px-2 text-right text-sm text-muted-foreground outline-hidden transition-colors hover:bg-muted/50 hover:text-foreground focus-visible:bg-muted/50 focus-visible:text-foreground disabled:cursor-not-allowed disabled:opacity-60",
            className,
          )}
          data-testid={testId}
          disabled={disabled}
          onClick={(event) => event.stopPropagation()}
          type="button"
        >
          <span className="min-w-0 truncate">{selectedLabel}</span>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => event.preventDefault()}
        sideOffset={4}
        style={{
          minWidth: "max(var(--radix-dropdown-menu-trigger-width), 13rem)",
        }}
      >
        <DropdownMenuRadioGroup onValueChange={onValueChange} value={value}>
          {options.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
