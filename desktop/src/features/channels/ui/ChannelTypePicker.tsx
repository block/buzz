import { ChevronDown, ClockFading, Hash } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function ChannelTypePicker({
  align = "start",
  ariaLabel,
  className,
  disabled,
  onOpenChange,
  onTemporaryChange,
  open,
  temporary,
  temporaryOptionAriaLabel = "Temporary channel",
  testId,
}: {
  align?: React.ComponentProps<typeof DropdownMenuContent>["align"];
  ariaLabel?: string;
  className?: string;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
  onTemporaryChange: (temporary: boolean) => void;
  open?: boolean;
  temporary: boolean;
  temporaryOptionAriaLabel?: string;
  testId?: string;
}) {
  const [internalOpen, setInternalOpen] = React.useState(false);
  const pickerOpen = open ?? internalOpen;
  const setPickerOpen = onOpenChange ?? setInternalOpen;
  const label = temporary ? "Temporary" : "Ongoing";
  const Icon = temporary ? ClockFading : Hash;

  function selectType(nextType: string) {
    onTemporaryChange(nextType === "temporary");
    setPickerOpen(false);
  }

  return (
    <DropdownMenu modal={false} onOpenChange={setPickerOpen} open={pickerOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label={ariaLabel ?? `Channel type: ${label}`}
          className={cn(
            "h-9 w-fit px-2.5 text-sm font-medium text-foreground hover:bg-muted/50",
            className,
          )}
          data-testid={testId}
          disabled={disabled}
          type="button"
          variant="ghost"
        >
          <Icon className="h-4 w-4" />
          {label}
          <ChevronDown className="h-4 w-4 text-muted-foreground/70" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align={align}
        onCloseAutoFocus={(event) => event.preventDefault()}
        style={{
          minWidth: "var(--radix-dropdown-menu-trigger-width)",
        }}
      >
        <DropdownMenuRadioGroup
          onValueChange={selectType}
          value={temporary ? "temporary" : "ongoing"}
        >
          <DropdownMenuRadioItem aria-label="Ongoing channel" value="ongoing">
            Ongoing
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem
            aria-label={temporaryOptionAriaLabel}
            value="temporary"
          >
            Temporary
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
