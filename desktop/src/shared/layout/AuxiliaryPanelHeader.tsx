import * as React from "react";
import { ArrowLeft, X } from "lucide-react";

import { channelChrome } from "@/shared/layout/chromeLayout";
import { AuxiliaryPanelContext } from "@/shared/layout/auxiliaryPanelContext";
import type { AuxiliaryPanelMode } from "@/shared/layout/auxiliaryPanelContext";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

export type { AuxiliaryPanelMode } from "@/shared/layout/auxiliaryPanelContext";
type AuxiliaryPanelHeaderProps = Omit<
  React.ComponentProps<"div">,
  "className"
> & {
  backdrop?: boolean;
  backdropSurface?: AuxiliaryPanelSurface;
  bordered?: boolean;
  density?: "comfortable" | "compact";
  inset?: "default" | "wide";
  mode?: AuxiliaryPanelMode;
  resizeBorder?: boolean;
  surface?: AuxiliaryPanelSurface;
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
  /**
   * Panel-specific control rendered ahead of the title, after any back button.
   *
   * A slot rather than a header-level feature because this header is shared by
   * every auxiliary panel and the controls that belong here are not: only the
   * thread has view modes. Each panel passes what it owns.
   */
  leading?: React.ReactNode;
  mode?: AuxiliaryPanelMode;
  onBack?: () => void;
};
type AuxiliaryPanelHeaderActionsProps = {
  children?: React.ReactNode;
  includeCloseAction?: boolean;
};
type AuxiliaryPanelHeaderTitleBlockProps = {
  subtitle?: React.ReactNode;
  subtitleTitle?: string;
  title: React.ReactNode;
};
type AuxiliaryPanelTitleProps = Omit<React.ComponentProps<"h2">, "className">;
type AuxiliaryPanelTitleContentProps = React.ComponentProps<"h2">;
type AuxiliaryPanelSurface = "default" | "soft" | "transparent";

const AUXILIARY_PANEL_HEADER_HEIGHT_CLASS = "pt-13";
const AUXILIARY_PANEL_CLOSE_LABEL = "Close panel";
const AUXILIARY_PANEL_CLOSE_TEST_ID = "auxiliary-panel-close";
const AUXILIARY_PANEL_RESIZE_BORDER_CLASS =
  "after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/auxiliary-panel-resize:after:bg-border/80 peer-focus-visible/auxiliary-panel-resize:after:bg-border/80";

export function getAuxiliaryPanelMode(
  isSplitLayout: boolean,
  isFloatingOverlay: boolean,
): AuxiliaryPanelMode {
  if (isSplitLayout) {
    return "docked";
  }

  return isFloatingOverlay ? "panel" : "single-panel";
}

function getAuxiliaryPanelSurfaceClass(surface: AuxiliaryPanelSurface) {
  if (surface === "transparent") {
    return "bg-transparent";
  }

  if (surface === "soft") {
    return "bg-background/75 backdrop-blur-md supports-[backdrop-filter]:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35";
  }

  return "bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55";
}

type AuxiliaryPanelHeaderBackdropProps = {
  surface: Exclude<AuxiliaryPanelSurface, "transparent">;
};

function AuxiliaryPanelHeaderBackdrop({
  surface,
}: AuxiliaryPanelHeaderBackdropProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 z-40 h-13",
        getAuxiliaryPanelSurfaceClass(surface),
      )}
    />
  );
}

/** Title/action row for right auxiliary panels across docked and standalone modes. */
export function AuxiliaryPanelHeader({
  backdrop = false,
  backdropSurface = "default",
  bordered = false,
  children,
  density = "comfortable",
  inset = "default",
  mode,
  resizeBorder = false,
  surface = "default",
  transparent,
  ...props
}: AuxiliaryPanelHeaderProps) {
  const panelContext = React.useContext(AuxiliaryPanelContext);
  const resolvedMode = mode ?? panelContext?.mode ?? "docked";
  const resolvedTransparent =
    transparent ?? panelContext?.transparentChrome ?? false;

  if (resolvedMode !== "docked") {
    const isSinglePanel = resolvedMode === "single-panel";
    const effectiveSurface = resolvedTransparent ? "transparent" : surface;

    return (
      <>
        {backdrop && backdropSurface !== "transparent" ? (
          <AuxiliaryPanelHeaderBackdrop surface={backdropSurface} />
        ) : null}
        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanel
              ? cn(
                  "relative z-41 -mb-13 min-h-13 shrink-0 gap-2.5 px-4 py-2 sm:pr-3",
                  inset === "wide" && "sm:pl-6",
                  resizeBorder && AUXILIARY_PANEL_RESIZE_BORDER_CLASS,
                  getAuxiliaryPanelSurfaceClass(effectiveSurface),
                )
              : resizeBorder
                ? cn(
                    "absolute inset-x-0 top-0 z-50 min-h-13 gap-3 bg-transparent px-3 py-2",
                    AUXILIARY_PANEL_RESIZE_BORDER_CLASS,
                  )
                : cn(
                    "relative z-50 shrink-0 gap-3",
                    density === "compact"
                      ? "min-h-11 px-3 py-1.5 text-left shadow-none"
                      : "min-h-13 px-5 py-2",
                    inset === "wide" && "sm:pl-6",
                    bordered && "border-b border-border/35",
                    getAuxiliaryPanelSurfaceClass(effectiveSurface),
                  ),
          )}
          data-tauri-drag-region
          {...props}
        >
          {renderAuxiliaryPanelHeaderContent(children)}
        </div>
      </>
    );
  }

  return (
    <div
      className={cn(
        "pointer-events-none relative z-40 overflow-visible",
        getAuxiliaryPanelSurfaceClass(
          resolvedTransparent ? "transparent" : surface,
        ),
        channelChrome.negativeMargin,
      )}
      {...props}
    >
      <div
        className="pointer-events-auto relative z-40 shrink-0 cursor-default select-none py-2 pl-5 pr-3"
        data-tauri-drag-region
      >
        <div className="flex h-9 min-w-0 items-center gap-2.5">
          {renderAuxiliaryPanelHeaderContent(children)}
        </div>
      </div>
    </div>
  );
}

function renderAuxiliaryPanelHeaderContent(children: React.ReactNode) {
  const { foundActions, content } = attachCloseActionToHeaderActions(children);

  if (foundActions) {
    return content;
  }

  return (
    <>
      {children}
      <AuxiliaryPanelHeaderActions includeCloseAction />
    </>
  );
}

function attachCloseActionToHeaderActions(children: React.ReactNode): {
  content: React.ReactNode;
  foundActions: boolean;
} {
  let foundActions = false;

  const content = React.Children.map(children, (child) => {
    if (!React.isValidElement<AuxiliaryPanelHeaderActionsProps>(child)) {
      return child;
    }

    if (child.type === AuxiliaryPanelHeaderActions) {
      foundActions = true;
      return React.cloneElement(child, { includeCloseAction: true });
    }

    if (child.type === React.Fragment) {
      const nested = attachCloseActionToHeaderActions(child.props.children);

      if (!nested.foundActions) {
        return child;
      }

      foundActions = true;
      return React.cloneElement(child, undefined, nested.content);
    }

    return child;
  });

  return { content, foundActions };
}

export function getAuxiliaryPanelBodyClass({
  isFloatingOverlay = false,
  isSplitLayout = false,
  mode,
}: {
  isFloatingOverlay?: boolean;
  isSplitLayout?: boolean;
  mode?: AuxiliaryPanelMode;
}) {
  const resolvedMode =
    mode ?? getAuxiliaryPanelMode(isSplitLayout, isFloatingOverlay);

  return cn(
    resolvedMode === "docked" && channelChrome.contentPadding,
    resolvedMode === "single-panel" && AUXILIARY_PANEL_HEADER_HEIGHT_CLASS,
  );
}

export function AuxiliaryPanelHeaderGroup({
  align = "center",
  backButtonAriaLabel = "Back",
  backButtonTestId,
  leading,
  mode,
  children,
  onBack,
  ...props
}: AuxiliaryPanelHeaderGroupProps) {
  const panelContext = React.useContext(AuxiliaryPanelContext);
  const resolvedMode = mode ?? panelContext?.mode ?? "docked";
  const isOverlayLayout = resolvedMode === "panel";

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
      {/*
       * Sits inside the header's normal inset rather than pulled flush like the
       * back button: a back button is chrome and belongs on the panel edge, but
       * this is a control acting on the panel's own content, so it reads as part
       * of the title row and lets the title shift right off it.
       */}
      {leading ? (
        <div className="flex shrink-0 items-center">{leading}</div>
      ) : null}
      {children}
    </div>
  );
}

export function AuxiliaryPanelHeaderActions({
  children,
  includeCloseAction = false,
}: AuxiliaryPanelHeaderActionsProps) {
  if (!children && !includeCloseAction) {
    return null;
  }

  return (
    <div className="ml-auto flex shrink-0 items-center gap-0.5">
      {children}
      {includeCloseAction ? <AuxiliaryPanelHeaderCloseAction /> : null}
    </div>
  );
}

function AuxiliaryPanelHeaderCloseAction() {
  const panelContext = React.useContext(AuxiliaryPanelContext);

  if (!panelContext?.onClose) {
    return null;
  }

  return (
    <Button
      aria-label={AUXILIARY_PANEL_CLOSE_LABEL}
      className="shrink-0"
      data-testid={AUXILIARY_PANEL_CLOSE_TEST_ID}
      onClick={panelContext.onClose}
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
