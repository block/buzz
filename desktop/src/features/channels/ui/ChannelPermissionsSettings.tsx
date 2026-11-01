// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { ChevronDown, Globe, Lock } from "lucide-react";

import type { ChannelVisibility } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { cn } from "@/shared/lib/cn";
import { SegmentedControl } from "@/shared/ui/segmented-control";

const VISIBILITY_OPTIONS = [
  {
    value: "private",
    get label() {
      return uiT("Private");
    },
    Icon: Lock,
  },
  {
    value: "open",
    get label() {
      return uiT("Public");
    },
    Icon: Globe,
  },
] as const;

export function ChannelPermissionsSettings({
  disabled,
  onVisibilityChange,
  testIdPrefix,
  visibility,
  variant = "dropdown",
}: {
  disabled?: boolean;
  onVisibilityChange: (visibility: ChannelVisibility) => void;
  testIdPrefix: string;
  visibility: ChannelVisibility;
  variant?: "dropdown" | "segmented";
}) {
  const visibilityLabel =
    visibility === "private" ? uiT("Private") : uiT("Public");

  return (
    <div
      className={cn(
        "flex min-h-12 items-center justify-between gap-4 rounded-xl border border-input bg-background px-3 py-3",
        disabled && variant === "dropdown" && "opacity-50",
      )}
      data-testid={`${testIdPrefix}-permissions-container`}
    >
      <span
        className={cn(
          "text-sm font-medium text-foreground",
          disabled && variant === "segmented" && "opacity-50",
        )}
      >
        {uiT("Visibility")}
      </span>
      {variant === "segmented" ? (
        <SegmentedControl
          disabled={disabled}
          legend={uiT("Visibility")}
          onValueChange={onVisibilityChange}
          optionTestIdPrefix={`${testIdPrefix}-permissions-option`}
          options={VISIBILITY_OPTIONS}
          testId={`${testIdPrefix}-permissions`}
          value={visibility}
        />
      ) : (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={uiT("Visibility: {0}", { p0: visibilityLabel })}
              className="-mr-2.5 ml-auto h-9 w-fit justify-end px-2.5 text-right text-sm font-medium text-foreground hover:bg-muted/50"
              data-testid={`${testIdPrefix}-permissions`}
              disabled={disabled}
              type="button"
              variant="ghost"
            >
              <span aria-live="polite" className="text-right">
                {visibilityLabel}
              </span>
              <ChevronDown className="size-4 shrink-0 text-muted-foreground/70" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            onCloseAutoFocus={(event) => event.preventDefault()}
            style={{
              minWidth: "var(--radix-dropdown-menu-trigger-width)",
            }}
          >
            <DropdownMenuRadioGroup
              onValueChange={(nextVisibility) =>
                onVisibilityChange(
                  nextVisibility === "private" ? "private" : "open",
                )
              }
              value={visibility}
            >
              <DropdownMenuRadioItem
                data-testid={`${testIdPrefix}-permissions-option-open`}
                value="open"
              >
                {uiT("Public")}
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem
                data-testid={`${testIdPrefix}-permissions-option-private`}
                value="private"
              >
                {uiT("Private")}
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}
