// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import { EllipsisVertical, EyeOff } from "lucide-react";
import { toast } from "sonner";

import { useAppShell } from "@/app/AppShellContext";
import {
  setLinkPreviewStyle,
  type LinkPreviewStyle,
  useLinkPreviewStyle,
} from "@/shared/lib/linkPreviewStylePreference";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const CONTROL_BUTTON_CLASS =
  "h-5 w-5 rounded-full text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/message:opacity-100 data-[state=open]:opacity-100";

const LINK_PREVIEW_STYLE_OPTIONS: {
  value: LinkPreviewStyle;
  label: string;
}[] = [
  {
    value: "rich",
    get label() {
      return uiT("Rich");
    },
  },
  {
    value: "compact",
    get label() {
      return uiT("Compact");
    },
  },
];

export function LinkPreviewControls({
  onRemove,
  placement = "right",
}: {
  onRemove?: () => void;
  placement?: "left" | "right";
}) {
  useUiLanguage();

  const style = useLinkPreviewStyle();
  const { onOpenSettings } = useAppShell();

  const handleStyleChange = (nextStyle: string) => {
    if (
      (nextStyle !== "rich" && nextStyle !== "compact") ||
      nextStyle === style
    ) {
      return;
    }

    setLinkPreviewStyle(nextStyle);
    toast.success(
      uiT("Link previews set to {0}.", {
        p0: nextStyle === "rich" ? "Rich" : "Compact",
      }),
      {
        action: onOpenSettings
          ? {
              get label() {
                return uiT("Appearance");
              },
              onClick: () => onOpenSettings("appearance"),
            }
          : undefined,
        get description() {
          return uiT(
            "You can always modify this and other settings in Appearance.",
          );
        },
      },
    );
  };

  return (
    <div
      className={cn(
        "absolute top-0 z-20 flex flex-col",
        placement === "left" ? "right-full" : "left-full ml-1",
      )}
    >
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={uiT("Link display settings")}
            className={CONTROL_BUTTON_CLASS}
            size="icon-xs"
            title={uiT("Link display settings")}
            type="button"
            variant="ghost"
          >
            <EllipsisVertical aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" side="right">
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>{uiT("Display")}</DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
              <DropdownMenuRadioGroup
                onValueChange={handleStyleChange}
                value={style}
              >
                {LINK_PREVIEW_STYLE_OPTIONS.map((option) => (
                  <DropdownMenuRadioItem
                    key={option.value}
                    value={option.value}
                  >
                    {option.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuSubContent>
          </DropdownMenuSub>
          {onRemove ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                onClick={onRemove}
              >
                <EyeOff aria-hidden="true" />
                {uiT("Remove preview")}
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
