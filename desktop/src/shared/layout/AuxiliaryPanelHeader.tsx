import type * as React from "react";
import { ArrowLeft, X } from "lucide-react";

import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type AuxiliaryPanelLayout = "overlay" | "split";
type AuxiliaryPanelHeaderProps = Omit<
  React.ComponentProps<"div">,
  "className"
> & {
  /** Render header content without its own backdrop for a shared parent chrome. */
  transparent?: boolean;
};
type AuxiliaryPanelHeaderGroupProps = Omit<
  React.ComponentProps<"div">,
  "className"
> & {
  align?: "center" | "start";
  backButtonAriaLabel?: string;
  backButtonTestId?: string;
  layout?: AuxiliaryPanelLayout;
  onBack?: () => void;
};
type AuxiliaryPanelHeaderActionsProps = {
  children: React.ReactNode;
};
type AuxiliaryPanelHeaderCloseButtonProps = {
  ariaLabel: string;
  onClose: () => void;
  onPointerDown?: React.PointerEventHandler<HTMLButtonElement>;
  testId?: string;
};
type AuxiliaryPanelHeaderTitleBlockProps = {
  subtitle?: React.ReactNode;
  subtitleTitle?: string;
  title: React.ReactNode;
};
type AuxiliaryPanelTitleProps = Omit<React.ComponentProps<"h2">, "className">;
type AuxiliaryPanelTitleContentProps = React.ComponentProps<"h2">;
type AuxiliaryPanelFloatingHeaderBackdropProps = {
  surface?: "default" | "soft";
};
type AuxiliaryPanelFloatingHeaderProps = Omit<
  React.ComponentProps<"div">,
  "className"
> & {
  children: React.ReactNode;
  resizeBorder?: boolean;
  singleColumn?: boolean;
  singleColumnInset?: "default" | "wide";
  surface?: "default" | "transparent";
};

/** Compact title/action row for right auxiliary panels in split layouts. */
export function AuxiliaryPanelHeader({
  children,
  transparent = false,
  ...props
}: AuxiliaryPanelHeaderProps) {
  return (
    <div
      className={cn(
        "pointer-events-none relative z-40 overflow-visible",
        transparent
          ? "bg-transparent"
          : "bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
        channelChrome.negativeMargin,
      )}
      {...props}
    >
      <div
        className="pointer-events-auto relative z-40 shrink-0 cursor-default select-none px-5 py-2"
        data-tauri-drag-region
      >
        <div className="flex h-9 min-w-0 items-center gap-2.5">{children}</div>
      </div>
    </div>
  );
}

export function getAuxiliaryPanelBodyClass({
  isSplitLayout = false,
  reserveFloatingHeader = false,
}: {
  isSplitLayout?: boolean;
  reserveFloatingHeader?: boolean;
}) {
  return cn(
    isSplitLayout && channelChrome.contentPadding,
    reserveFloatingHeader && "pt-[3.25rem]",
  );
}

export function AuxiliaryPanelFloatingHeaderBackdrop({
  surface = "default",
}: AuxiliaryPanelFloatingHeaderBackdropProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 z-40 h-[3.25rem]",
        surface === "soft"
          ? "bg-background/75 backdrop-blur-md supports-[backdrop-filter]:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35"
          : "bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
      )}
    />
  );
}

export function AuxiliaryPanelFloatingHeader({
  children,
  resizeBorder = false,
  singleColumn = false,
  singleColumnInset = "default",
  surface = "default",
  ...props
}: AuxiliaryPanelFloatingHeaderProps) {
  return (
    <div
      className={cn(
        "flex cursor-default select-none items-center",
        singleColumn
          ? cn(
              "relative z-[41] -mb-[3.25rem] min-h-[3.25rem] shrink-0 gap-2.5 px-4 py-2 sm:pr-3",
              singleColumnInset === "wide" && "sm:pl-6",
              surface === "transparent"
                ? "bg-transparent"
                : "bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
            )
          : resizeBorder
            ? "absolute inset-x-0 top-0 z-50 min-h-13 gap-3 bg-transparent px-3 py-2 after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80"
            : "relative z-50 min-h-[3.25rem] shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
      )}
      data-tauri-drag-region
      {...props}
    >
      {children}
    </div>
  );
}

export function AuxiliaryPanelHeaderGroup({
  align = "center",
  backButtonAriaLabel = "Back",
  backButtonTestId,
  layout = "split",
  children,
  onBack,
  ...props
}: AuxiliaryPanelHeaderGroupProps) {
  const isOverlayLayout = layout === "overlay";

  return (
    <div
      className={cn(
        "flex min-w-0 flex-1 gap-1.5",
        align === "start" ? "items-start" : "items-center",
      )}
      {...props}
    >
      {onBack ? (
        <Button
          aria-label={backButtonAriaLabel}
          // Header text needs a comfortable left inset in split layouts, but a
          // leading icon should visually sit closer to the panel edge. Overlay
          // headers already use compact row padding, so keep that button flush.
          className={cn("shrink-0", isOverlayLayout ? "ml-0" : "-ml-2")}
          data-testid={backButtonTestId}
          onClick={onBack}
          size="icon"
          type="button"
          variant={isOverlayLayout ? "ghost" : "outline"}
        >
          <ArrowLeft />
        </Button>
      ) : null}
      {children}
    </div>
  );
}

export function AuxiliaryPanelHeaderActions({
  children,
}: AuxiliaryPanelHeaderActionsProps) {
  return (
    <div className="ml-auto flex shrink-0 items-center gap-0.5">{children}</div>
  );
}

export function AuxiliaryPanelHeaderCloseButton({
  ariaLabel,
  onClose,
  onPointerDown,
  testId,
}: AuxiliaryPanelHeaderCloseButtonProps) {
  return (
    <Button
      aria-label={ariaLabel}
      data-testid={testId}
      onClick={onClose}
      onPointerDown={onPointerDown}
      size="icon"
      type="button"
      variant="ghost"
    >
      <X />
    </Button>
  );
}

export function AuxiliaryPanelHeaderTitleBlock({
  subtitle,
  subtitleTitle,
  title,
}: AuxiliaryPanelHeaderTitleBlockProps) {
  if (!subtitle) {
    return <AuxiliaryPanelTitle>{title}</AuxiliaryPanelTitle>;
  }

  return (
    <div className="min-w-0 flex-1">
      <AuxiliaryPanelTitleContent className="translate-y-0 leading-5">
        {title}
      </AuxiliaryPanelTitleContent>
      <p
        className="min-w-0 truncate font-mono text-2xs text-muted-foreground"
        title={subtitleTitle}
      >
        {subtitle}
      </p>
    </div>
  );
}

export function AuxiliaryPanelTitle(props: AuxiliaryPanelTitleProps) {
  return <AuxiliaryPanelTitleContent {...props} />;
}

function AuxiliaryPanelTitleContent({
  className,
  children,
  ...props
}: AuxiliaryPanelTitleContentProps) {
  return (
    <h2
      className={cn(
        "min-w-0 flex-1 translate-y-px truncate text-base font-semibold leading-6 tracking-tight",
        className,
      )}
      {...props}
    >
      {children}
    </h2>
  );
}
