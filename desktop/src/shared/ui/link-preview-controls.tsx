import { EllipsisVertical, EyeOff } from "lucide-react";
import { toast } from "sonner";

import { useTranslation } from "@/i18n";
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
}[] = [{ value: "rich" }, { value: "compact" }];

export function LinkPreviewControls({
  onRemove,
  placement = "right",
}: {
  onRemove?: () => void;
  placement?: "left" | "right";
}) {
  const { t } = useTranslation();
  const style = useLinkPreviewStyle();
  const { onOpenSettings } = useAppShell();

  const styleLabel = (value: string) =>
    value === "rich"
      ? t("shared.linkPreview.style.rich")
      : t("shared.linkPreview.style.compact");

  const handleStyleChange = (nextStyle: string) => {
    if (
      (nextStyle !== "rich" && nextStyle !== "compact") ||
      nextStyle === style
    ) {
      return;
    }

    setLinkPreviewStyle(nextStyle);
    toast.success(
      t("shared.linkPreview.style-changed", {
        style: styleLabel(nextStyle),
      }),
      {
        action: onOpenSettings
          ? {
              label: t("shared.linkPreview.appearance"),
              onClick: () => onOpenSettings("appearance"),
            }
          : undefined,
        description: t("shared.linkPreview.appearance-hint"),
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
            aria-label={t("shared.linkPreview.controls-aria")}
            className={CONTROL_BUTTON_CLASS}
            size="icon-xs"
            title={t("shared.linkPreview.controls-aria")}
            type="button"
            variant="ghost"
          >
            <EllipsisVertical aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" side="right">
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>
              {t("shared.linkPreview.display")}
            </DropdownMenuSubTrigger>
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
                    {styleLabel(option.value)}
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
                {t("shared.linkPreview.remove")}
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
